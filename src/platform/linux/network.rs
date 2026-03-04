///! Linux container networking — veth pairs, bridge, NAT via netlink.

use std::fs;

use crate::config::NetworkConfig;
use crate::error::{ContainerError, Result};
use super::syscall::*;

#[repr(C)]
struct NlMsgHdr {
    nlmsg_len: u32,
    nlmsg_type: u16,
    nlmsg_flags: u16,
    nlmsg_seq: u32,
    nlmsg_pid: u32,
}

#[repr(C)]
struct IfInfoMsg {
    ifi_family: u8,
    _pad: u8,
    ifi_type: u16,
    ifi_index: i32,
    ifi_flags: u32,
    ifi_change: u32,
}

#[repr(C)]
struct IfAddrMsg {
    ifa_family: u8,
    ifa_prefixlen: u8,
    ifa_flags: u8,
    ifa_scope: u8,
    ifa_index: u32,
}

#[repr(C)]
struct RtAttr {
    rta_len: u16,
    rta_type: u16,
}

#[repr(C)]
struct SockAddrNl {
    nl_family: u16,
    nl_pad: u16,
    nl_pid: u32,
    nl_groups: u32,
}

fn rta_align(len: usize) -> usize { (len + 3) & !3 }

pub fn setup_container_network(config: &NetworkConfig, container_pid: u32, name: &str) -> Result<()> {
    if !config.enabled { return Ok(()); }
    let bridge = "holy0";
    let veth_host = format!("veth_{}", &name[..name.len().min(8)]);
    create_bridge(bridge)?;
    set_interface_ip(bridge, &config.bridge_ip, config.subnet_bits)?;
    set_interface_up(bridge)?;
    create_veth_pair(&veth_host, "eth0")?;
    set_interface_master(&veth_host, bridge)?;
    set_interface_up(&veth_host)?;
    move_to_netns("eth0", container_pid)?;
    let _ = fs::write("/proc/sys/net/ipv4/ip_forward", "1");
    Ok(())
}

fn open_netlink_socket() -> Result<i32> {
    let fd = unsafe { socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE) };
    if fd < 0 { return Err(crate::error::syscall_error("socket(NETLINK)")); }
    let addr = SockAddrNl { nl_family: AF_NETLINK as u16, nl_pad: 0, nl_pid: 0, nl_groups: 0 };
    let ret = unsafe { bind(fd, &addr as *const _ as *const u8, std::mem::size_of::<SockAddrNl>() as u32) };
    if ret < 0 { unsafe { close(fd); } return Err(crate::error::syscall_error("bind(NETLINK)")); }
    Ok(fd)
}

fn send_netlink(sock: i32, msg: &[u8]) -> Result<()> {
    let addr = SockAddrNl { nl_family: AF_NETLINK as u16, nl_pad: 0, nl_pid: 0, nl_groups: 0 };
    let ret = unsafe { sendto(sock, msg.as_ptr(), msg.len(), 0, &addr as *const _ as *const u8, std::mem::size_of::<SockAddrNl>() as u32) };
    if ret < 0 { return Err(crate::error::syscall_error("sendto")); }
    let mut buf = [0u8; 4096];
    let mut alen = std::mem::size_of::<SockAddrNl>() as u32;
    let n = unsafe { recvfrom(sock, buf.as_mut_ptr(), buf.len(), 0, std::ptr::null_mut(), &mut alen) };
    if n < 0 { return Err(crate::error::syscall_error("recvfrom")); }
    if n as usize >= std::mem::size_of::<NlMsgHdr>() {
        let hdr = unsafe { &*(buf.as_ptr() as *const NlMsgHdr) };
        if hdr.nlmsg_type == 2 {
            let off = std::mem::size_of::<NlMsgHdr>();
            if n as usize >= off + 4 {
                let e = i32::from_ne_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]]);
                if e < 0 { return Err(ContainerError::Syscall { call: "netlink", code: -e, detail: format!("errno {}", -e) }); }
            }
        }
    }
    Ok(())
}

fn create_bridge(name: &str) -> Result<()> {
    let sock = open_netlink_socket()?;
    let mut buf = [0u8; 512];
    let hs = std::mem::size_of::<NlMsgHdr>();
    let is = std::mem::size_of::<IfInfoMsg>();
    let rs = std::mem::size_of::<RtAttr>();
    let nb = name.as_bytes();
    let rn = rta_align(rs + nb.len() + 1);
    let kind = b"bridge\0";
    let rk = rta_align(rs + kind.len());
    let rl = rta_align(rs + rk);
    let total = hs + is + rn + rl;
    let mut o = 0;
    let h = NlMsgHdr { nlmsg_len: total as u32, nlmsg_type: RTM_NEWLINK, nlmsg_flags: NLM_F_REQUEST|NLM_F_CREATE|NLM_F_EXCL|NLM_F_ACK, nlmsg_seq: 1, nlmsg_pid: 0 };
    unsafe { std::ptr::copy_nonoverlapping(&h as *const _ as *const u8, buf.as_mut_ptr(), hs); } o += hs;
    let ifi = IfInfoMsg { ifi_family: 0, _pad: 0, ifi_type: 0, ifi_index: 0, ifi_flags: 0, ifi_change: 0 };
    unsafe { std::ptr::copy_nonoverlapping(&ifi as *const _ as *const u8, buf.as_mut_ptr().add(o), is); } o += is;
    let ra = RtAttr { rta_len: (rs + nb.len() + 1) as u16, rta_type: IFLA_IFNAME };
    unsafe { std::ptr::copy_nonoverlapping(&ra as *const _ as *const u8, buf.as_mut_ptr().add(o), rs); } o += rs;
    buf[o..o+nb.len()].copy_from_slice(nb); o = hs + is + rn;
    let rl_a = RtAttr { rta_len: (rs + rk) as u16, rta_type: IFLA_LINKINFO };
    unsafe { std::ptr::copy_nonoverlapping(&rl_a as *const _ as *const u8, buf.as_mut_ptr().add(o), rs); } o += rs;
    let rk_a = RtAttr { rta_len: (rs + kind.len()) as u16, rta_type: IFLA_INFO_KIND };
    unsafe { std::ptr::copy_nonoverlapping(&rk_a as *const _ as *const u8, buf.as_mut_ptr().add(o), rs); } o += rs;
    buf[o..o+kind.len()].copy_from_slice(kind);
    let r = send_netlink(sock, &buf[..total]);
    unsafe { close(sock); }
    match r { Ok(()) => Ok(()), Err(ContainerError::Syscall{code,..}) if code==17 => Ok(()), Err(e) => Err(e) }
}

fn create_veth_pair(n1: &str, n2: &str) -> Result<()> {
    let sock = open_netlink_socket()?;
    let mut buf = [0u8; 1024];
    let hs = std::mem::size_of::<NlMsgHdr>();
    let is = std::mem::size_of::<IfInfoMsg>();
    let rs = std::mem::size_of::<RtAttr>();
    let n1b = n1.as_bytes(); let n2b = n2.as_bytes();
    let rn1 = rta_align(rs+n1b.len()+1);
    let kind = b"veth\0";
    let rk = rta_align(rs+kind.len());
    let pn = rta_align(rs+n2b.len()+1);
    let rid = rta_align(rs + is + pn);
    let rli = rta_align(rs + rk + rid);
    let total = hs + is + rn1 + rli;
    let mut o = 0;
    let h = NlMsgHdr { nlmsg_len: total as u32, nlmsg_type: RTM_NEWLINK, nlmsg_flags: NLM_F_REQUEST|NLM_F_CREATE|NLM_F_EXCL|NLM_F_ACK, nlmsg_seq: 2, nlmsg_pid: 0 };
    unsafe { std::ptr::copy_nonoverlapping(&h as *const _ as *const u8, buf.as_mut_ptr(), hs); } o+=hs;
    let ifi = IfInfoMsg{ifi_family:0,_pad:0,ifi_type:0,ifi_index:0,ifi_flags:0,ifi_change:0};
    unsafe { std::ptr::copy_nonoverlapping(&ifi as *const _ as *const u8, buf.as_mut_ptr().add(o), is); } o+=is;
    let ra=RtAttr{rta_len:(rs+n1b.len()+1) as u16,rta_type:IFLA_IFNAME};
    unsafe{std::ptr::copy_nonoverlapping(&ra as *const _ as *const u8,buf.as_mut_ptr().add(o),rs);} o+=rs;
    buf[o..o+n1b.len()].copy_from_slice(n1b); o=hs+is+rn1;
    let rli_a=RtAttr{rta_len:(rs+rk+rid)as u16,rta_type:IFLA_LINKINFO};
    unsafe{std::ptr::copy_nonoverlapping(&rli_a as *const _ as *const u8,buf.as_mut_ptr().add(o),rs);} o+=rs;
    let rk_a=RtAttr{rta_len:(rs+kind.len())as u16,rta_type:IFLA_INFO_KIND};
    unsafe{std::ptr::copy_nonoverlapping(&rk_a as *const _ as *const u8,buf.as_mut_ptr().add(o),rs);} o+=rs;
    buf[o..o+kind.len()].copy_from_slice(kind); o=hs+is+rn1+rs+rk;
    let rid_a=RtAttr{rta_len:(rs+is+pn)as u16,rta_type:IFLA_INFO_DATA};
    unsafe{std::ptr::copy_nonoverlapping(&rid_a as *const _ as *const u8,buf.as_mut_ptr().add(o),rs);} o+=rs;
    let pifi=IfInfoMsg{ifi_family:0,_pad:0,ifi_type:0,ifi_index:0,ifi_flags:0,ifi_change:0};
    unsafe{std::ptr::copy_nonoverlapping(&pifi as *const _ as *const u8,buf.as_mut_ptr().add(o),is);} o+=is;
    let pna=RtAttr{rta_len:(rs+n2b.len()+1)as u16,rta_type:IFLA_IFNAME};
    unsafe{std::ptr::copy_nonoverlapping(&pna as *const _ as *const u8,buf.as_mut_ptr().add(o),rs);} o+=rs;
    buf[o..o+n2b.len()].copy_from_slice(n2b);
    let r=send_netlink(sock,&buf[..total]);
    unsafe{close(sock);}
    r
}

fn set_interface_ip(name: &str, ip: &str, prefix: u8) -> Result<()> {
    let idx = get_interface_index(name)?;
    let ipb = parse_ipv4(ip)?;
    let sock = open_netlink_socket()?;
    let hs=std::mem::size_of::<NlMsgHdr>(); let as_=std::mem::size_of::<IfAddrMsg>(); let rs=std::mem::size_of::<RtAttr>();
    let ra=rta_align(rs+4); let total=hs+as_+ra+ra;
    let mut buf=[0u8;256]; let mut o=0;
    let h=NlMsgHdr{nlmsg_len:total as u32,nlmsg_type:RTM_NEWADDR,nlmsg_flags:NLM_F_REQUEST|NLM_F_CREATE|NLM_F_EXCL|NLM_F_ACK,nlmsg_seq:3,nlmsg_pid:0};
    unsafe{std::ptr::copy_nonoverlapping(&h as *const _ as *const u8,buf.as_mut_ptr(),hs);} o+=hs;
    let ifa=IfAddrMsg{ifa_family:AF_INET as u8,ifa_prefixlen:prefix,ifa_flags:0,ifa_scope:0,ifa_index:idx as u32};
    unsafe{std::ptr::copy_nonoverlapping(&ifa as *const _ as *const u8,buf.as_mut_ptr().add(o),as_);} o+=as_;
    let a1=RtAttr{rta_len:(rs+4)as u16,rta_type:IFA_ADDRESS};
    unsafe{std::ptr::copy_nonoverlapping(&a1 as *const _ as *const u8,buf.as_mut_ptr().add(o),rs);} o+=rs;
    buf[o..o+4].copy_from_slice(&ipb); o=hs+as_+ra;
    let a2=RtAttr{rta_len:(rs+4)as u16,rta_type:IFA_LOCAL};
    unsafe{std::ptr::copy_nonoverlapping(&a2 as *const _ as *const u8,buf.as_mut_ptr().add(o),rs);} o+=rs;
    buf[o..o+4].copy_from_slice(&ipb);
    let r=send_netlink(sock,&buf[..total]);
    unsafe{close(sock);}
    match r { Ok(())=>Ok(()), Err(ContainerError::Syscall{code,..}) if code==17=>Ok(()), Err(e)=>Err(e) }
}

fn set_interface_up(name: &str) -> Result<()> {
    let idx = get_interface_index(name)?;
    let sock = open_netlink_socket()?;
    let hs=std::mem::size_of::<NlMsgHdr>(); let is=std::mem::size_of::<IfInfoMsg>();
    let total=hs+is; let mut buf=[0u8;128];
    let h=NlMsgHdr{nlmsg_len:total as u32,nlmsg_type:RTM_SETLINK,nlmsg_flags:NLM_F_REQUEST|NLM_F_ACK,nlmsg_seq:4,nlmsg_pid:0};
    unsafe{std::ptr::copy_nonoverlapping(&h as *const _ as *const u8,buf.as_mut_ptr(),hs);}
    let ifi=IfInfoMsg{ifi_family:0,_pad:0,ifi_type:0,ifi_index:idx,ifi_flags:IFF_UP,ifi_change:IFF_UP};
    unsafe{std::ptr::copy_nonoverlapping(&ifi as *const _ as *const u8,buf.as_mut_ptr().add(hs),is);}
    let r=send_netlink(sock,&buf[..total]); unsafe{close(sock);} r
}

fn set_interface_master(name: &str, bridge: &str) -> Result<()> {
    let idx=get_interface_index(name)?; let bidx=get_interface_index(bridge)?;
    let sock=open_netlink_socket()?;
    let hs=std::mem::size_of::<NlMsgHdr>(); let is=std::mem::size_of::<IfInfoMsg>(); let rs=std::mem::size_of::<RtAttr>();
    let rt=rta_align(rs+4); let total=hs+is+rt; let mut buf=[0u8;128]; let mut o=0;
    let h=NlMsgHdr{nlmsg_len:total as u32,nlmsg_type:RTM_SETLINK,nlmsg_flags:NLM_F_REQUEST|NLM_F_ACK,nlmsg_seq:5,nlmsg_pid:0};
    unsafe{std::ptr::copy_nonoverlapping(&h as *const _ as *const u8,buf.as_mut_ptr(),hs);} o+=hs;
    let ifi=IfInfoMsg{ifi_family:0,_pad:0,ifi_type:0,ifi_index:idx,ifi_flags:0,ifi_change:0};
    unsafe{std::ptr::copy_nonoverlapping(&ifi as *const _ as *const u8,buf.as_mut_ptr().add(o),is);} o+=is;
    let a=RtAttr{rta_len:(rs+4)as u16,rta_type:IFLA_MASTER};
    unsafe{std::ptr::copy_nonoverlapping(&a as *const _ as *const u8,buf.as_mut_ptr().add(o),rs);} o+=rs;
    buf[o..o+4].copy_from_slice(&(bidx as u32).to_ne_bytes());
    let r=send_netlink(sock,&buf[..total]); unsafe{close(sock);} r
}

fn move_to_netns(name: &str, pid: u32) -> Result<()> {
    let idx=get_interface_index(name)?;
    let sock=open_netlink_socket()?;
    let hs=std::mem::size_of::<NlMsgHdr>(); let is=std::mem::size_of::<IfInfoMsg>(); let rs=std::mem::size_of::<RtAttr>();
    let rt=rta_align(rs+4); let total=hs+is+rt; let mut buf=[0u8;128]; let mut o=0;
    let h=NlMsgHdr{nlmsg_len:total as u32,nlmsg_type:RTM_SETLINK,nlmsg_flags:NLM_F_REQUEST|NLM_F_ACK,nlmsg_seq:6,nlmsg_pid:0};
    unsafe{std::ptr::copy_nonoverlapping(&h as *const _ as *const u8,buf.as_mut_ptr(),hs);} o+=hs;
    let ifi=IfInfoMsg{ifi_family:0,_pad:0,ifi_type:0,ifi_index:idx,ifi_flags:0,ifi_change:0};
    unsafe{std::ptr::copy_nonoverlapping(&ifi as *const _ as *const u8,buf.as_mut_ptr().add(o),is);} o+=is;
    let a=RtAttr{rta_len:(rs+4)as u16,rta_type:IFLA_NET_NS_PID};
    unsafe{std::ptr::copy_nonoverlapping(&a as *const _ as *const u8,buf.as_mut_ptr().add(o),rs);} o+=rs;
    buf[o..o+4].copy_from_slice(&pid.to_ne_bytes());
    let r=send_netlink(sock,&buf[..total]); unsafe{close(sock);} r
}

fn get_interface_index(name: &str) -> Result<i32> {
    const SIOCGIFINDEX: u64 = 0x8933;
    let sock = unsafe { socket(AF_INET, SOCK_DGRAM, 0) };
    if sock < 0 { return Err(crate::error::syscall_error("socket")); }
    let mut ifr = [0u8; 40];
    let nb = name.as_bytes();
    ifr[..nb.len().min(15)].copy_from_slice(&nb[..nb.len().min(15)]);
    let ret = unsafe { ioctl(sock, SIOCGIFINDEX, ifr.as_mut_ptr()) };
    unsafe { close(sock); }
    if ret < 0 { return Err(crate::error::syscall_error("ioctl(SIOCGIFINDEX)")); }
    Ok(i32::from_ne_bytes([ifr[16],ifr[17],ifr[18],ifr[19]]))
}

fn parse_ipv4(ip: &str) -> Result<[u8; 4]> {
    let p: Vec<&str> = ip.split('.').collect();
    if p.len() != 4 { return Err(ContainerError::Network(format!("bad ip: {}", ip))); }
    let mut b = [0u8; 4];
    for (i, s) in p.iter().enumerate() {
        b[i] = s.parse().map_err(|_| ContainerError::Network(format!("bad octet: {}", s)))?;
    }
    Ok(b)
}
