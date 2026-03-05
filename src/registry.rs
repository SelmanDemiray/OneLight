///! Docker Registry HTTP API v2 client — pull images from Docker Hub and other registries.
///! Handles authentication (bearer tokens), manifest fetching, and layer downloading.
///! Zero third-party dependencies.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{ContainerError, Result};
use crate::http;
use crate::json::{self, JsonValue};
use crate::gzip;
use crate::image;

// ─── Registry Types ─────────────────────────────────────────────────────────

/// Parsed image reference: registry/namespace/name:tag
#[derive(Debug, Clone)]
pub struct ImageRef {
    pub registry: String,
    pub namespace: String,
    pub name: String,
    pub tag: String,
}

impl ImageRef {
    /// Parse an image reference like "ubuntu:22.04" or "registry.io/myapp:latest"
    pub fn parse(reference: &str) -> Result<ImageRef> {
        let (image_part, tag) = match reference.rfind(':') {
            // Make sure it's a tag separator, not part of a registry port
            Some(i) if !reference[i+1..].contains('/') => {
                (&reference[..i], reference[i+1..].to_string())
            }
            _ => (reference, "latest".to_string()),
        };

        let parts: Vec<&str> = image_part.split('/').collect();

        let (registry, namespace, name) = match parts.len() {
            1 => ("registry-1.docker.io".to_string(), "library".to_string(), parts[0].to_string()),
            2 => {
                // Could be registry/image or namespace/image
                if parts[0].contains('.') || parts[0].contains(':') {
                    (parts[0].to_string(), "library".to_string(), parts[1].to_string())
                } else {
                    ("registry-1.docker.io".to_string(), parts[0].to_string(), parts[1].to_string())
                }
            }
            3 => (parts[0].to_string(), parts[1].to_string(), parts[2].to_string()),
            _ => return Err(ContainerError::Config(format!("invalid image reference: {}", reference))),
        };

        Ok(ImageRef { registry, namespace, name, tag })
    }

    /// Full repository path (e.g., "library/ubuntu")
    pub fn repository(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }

    /// Display name
    pub fn display(&self) -> String {
        if self.namespace == "library" {
            format!("{}:{}", self.name, self.tag)
        } else {
            format!("{}/{}:{}", self.namespace, self.name, self.tag)
        }
    }
}

/// Layer descriptor from the manifest
#[derive(Debug, Clone)]
pub struct LayerDescriptor {
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

/// Image manifest
#[derive(Debug)]
pub struct ImageManifest {
    pub config_digest: String,
    pub layers: Vec<LayerDescriptor>,
}

// ─── Authentication ─────────────────────────────────────────────────────────

/// Get a bearer token for the Docker Hub registry.
fn get_auth_token(image_ref: &ImageRef) -> Result<String> {
    let repo = image_ref.repository();

    // Docker Hub uses token auth at auth.docker.io
    let auth_url = if image_ref.registry == "registry-1.docker.io" {
        format!(
            "https://auth.docker.io/token?service=registry.docker.io&scope=repository:{}:pull",
            repo
        )
    } else {
        // Other registries may use different auth endpoints
        // Try the /v2/ endpoint to discover auth requirements
        let check_url = format!("https://{}/v2/", image_ref.registry);
        let resp = http::get(&check_url, &[])?;
        if resp.status == 401 {
            if let Some(www_auth) = resp.header("www-authenticate") {
                if let Some(realm) = extract_auth_param(www_auth, "realm") {
                    let service = extract_auth_param(www_auth, "service").unwrap_or_default();
                    format!("{}?service={}&scope=repository:{}:pull", realm, service, repo)
                } else {
                    return Err(ContainerError::Network("cannot parse auth challenge".into()));
                }
            } else {
                return Err(ContainerError::Network("registry requires auth but no WWW-Authenticate".into()));
            }
        } else {
            // No auth needed
            return Ok(String::new());
        }
    };

    let resp = http::get(&auth_url, &[])?;
    if resp.status != 200 {
        return Err(ContainerError::Network(format!(
            "auth token request failed: HTTP {}", resp.status
        )));
    }

    let body = resp.body_string();
    let json_val = json::parse(&body)
        .map_err(|e| ContainerError::Network(format!("parse auth response: {}", e)))?;

    json_val.get("token")
        .or_else(|| json_val.get("access_token"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ContainerError::Network("no token in auth response".into()))
}

fn extract_auth_param(header: &str, param: &str) -> Option<String> {
    let search = format!("{}=\"", param);
    if let Some(start) = header.find(&search) {
        let value_start = start + search.len();
        if let Some(end) = header[value_start..].find('"') {
            return Some(header[value_start..value_start + end].to_string());
        }
    }
    None
}

// ─── Registry API ───────────────────────────────────────────────────────────

fn registry_get(url: &str, token: &str) -> Result<http::HttpResponse> {
    let mut headers = vec![
        ("Accept", "application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json"),
    ];

    if !token.is_empty() {
        headers.push(("Authorization", &format!("Bearer {}", token)));
    }

    // Need to handle the borrow issue
    let auth_header;
    let final_headers: Vec<(&str, &str)> = if !token.is_empty() {
        auth_header = format!("Bearer {}", token);
        vec![
            ("Accept", "application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json"),
            ("Authorization", &auth_header),
        ]
    } else {
        vec![
            ("Accept", "application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json"),
        ]
    };

    http::get_follow_redirects(url, &final_headers, 5)
}

/// Fetch the image manifest from the registry.
fn fetch_manifest(image_ref: &ImageRef, token: &str) -> Result<ImageManifest> {
    let url = format!(
        "https://{}/v2/{}/manifests/{}",
        image_ref.registry, image_ref.repository(), image_ref.tag
    );

    let resp = registry_get(&url, token)?;

    if resp.status == 404 {
        return Err(ContainerError::NotFound(format!("image {}", image_ref.display())));
    }
    if resp.status != 200 {
        return Err(ContainerError::Network(format!(
            "manifest fetch failed: HTTP {} — {}", resp.status, resp.body_string()
        )));
    }

    let body = resp.body_string();
    let manifest = json::parse(&body)
        .map_err(|e| ContainerError::Network(format!("parse manifest: {}", e)))?;

    // Check if this is a manifest list (multi-arch) — need to resolve to our platform
    let media_type = manifest.get("mediaType")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if media_type.contains("manifest.list") || manifest.get("manifests").is_some() {
        // This is a manifest list — find the amd64/linux manifest
        return resolve_manifest_list(image_ref, token, &manifest);
    }

    // Parse regular manifest
    parse_manifest(&manifest)
}

fn resolve_manifest_list(image_ref: &ImageRef, token: &str, list: &JsonValue) -> Result<ImageManifest> {
    let manifests = list.get("manifests")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ContainerError::Network("no manifests in manifest list".into()))?;

    // Find amd64/linux manifest
    for m in manifests {
        if let Some(platform) = m.get("platform") {
            let arch = platform.get("architecture").and_then(|v| v.as_str()).unwrap_or("");
            let os = platform.get("os").and_then(|v| v.as_str()).unwrap_or("");

            if arch == "amd64" && os == "linux" {
                let digest = m.get("digest")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ContainerError::Network("no digest in manifest list entry".into()))?;

                // Fetch the actual manifest by digest
                let url = format!(
                    "https://{}/v2/{}/manifests/{}",
                    image_ref.registry, image_ref.repository(), digest
                );
                let resp = registry_get(&url, token)?;
                if resp.status != 200 {
                    return Err(ContainerError::Network(format!(
                        "manifest fetch by digest failed: HTTP {}", resp.status
                    )));
                }
                let body = resp.body_string();
                let manifest = json::parse(&body)
                    .map_err(|e| ContainerError::Network(format!("parse manifest: {}", e)))?;
                return parse_manifest(&manifest);
            }
        }
    }

    Err(ContainerError::Network("no amd64/linux manifest found in manifest list".into()))
}

fn parse_manifest(manifest: &JsonValue) -> Result<ImageManifest> {
    let config_digest = manifest.get("config")
        .and_then(|c| c.get("digest"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let layers_arr = manifest.get("layers")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ContainerError::Network("no layers in manifest".into()))?;

    let mut layers = Vec::new();
    for layer in layers_arr {
        let media_type = layer.get("mediaType")
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream")
            .to_string();
        let digest = layer.get("digest")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ContainerError::Network("layer missing digest".into()))?
            .to_string();
        let size = layer.get("size")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as u64;

        layers.push(LayerDescriptor { media_type, digest, size });
    }

    Ok(ImageManifest { config_digest, layers })
}

// ─── Image Pull ─────────────────────────────────────────────────────────────

/// Pull an image from a registry and extract it to local storage.
pub fn pull_image(reference: &str) -> Result<PathBuf> {
    let image_ref = ImageRef::parse(reference)?;

    println!("[*] Pulling {}...", image_ref.display());
    println!("    Registry: {}", image_ref.registry);
    println!("    Repository: {}", image_ref.repository());

    // Get auth token
    println!("[*] Authenticating...");
    let token = get_auth_token(&image_ref)?;
    println!("    Token acquired.");

    // Fetch manifest
    println!("[*] Fetching manifest...");
    let manifest = fetch_manifest(&image_ref, &token)?;
    println!("    Found {} layers.", manifest.layers.len());

    // Create local storage directory
    let image_dir = image_storage_dir(&image_ref);
    let layers_dir = image_dir.join("layers");
    fs::create_dir_all(&layers_dir)
        .map_err(|e| ContainerError::Filesystem(format!("create image dir: {}", e)))?;

    // Download and extract each layer
    let rootfs_dir = image_dir.join("rootfs");
    fs::create_dir_all(&rootfs_dir)
        .map_err(|e| ContainerError::Filesystem(format!("create rootfs dir: {}", e)))?;

    for (i, layer) in manifest.layers.iter().enumerate() {
        println!("[*] Layer {}/{}: {} ({} bytes)",
            i + 1, manifest.layers.len(),
            &layer.digest[..19], layer.size);

        let layer_file = layers_dir.join(layer.digest.replace(":", "_"));

        if layer_file.exists() {
            println!("    Already downloaded, skipping.");
        } else {
            // Download the layer blob
            let blob_url = format!(
                "https://{}/v2/{}/blobs/{}",
                image_ref.registry, image_ref.repository(), layer.digest
            );

            let auth_header = format!("Bearer {}", token);
            let headers: Vec<(&str, &str)> = if !token.is_empty() {
                vec![("Authorization", &auth_header)]
            } else {
                vec![]
            };

            let resp = http::get_follow_redirects(&blob_url, &headers, 5)?;
            if resp.status != 200 {
                return Err(ContainerError::Network(format!(
                    "layer download failed: HTTP {}", resp.status
                )));
            }

            fs::write(&layer_file, &resp.body)
                .map_err(|e| ContainerError::Filesystem(format!("write layer: {}", e)))?;

            println!("    Downloaded {} bytes.", resp.body.len());
        }

        // Extract the layer (it's a tar.gz)
        println!("    Extracting...");
        extract_layer(&layer_file, &rootfs_dir, &layer.media_type)?;
    }

    // Save manifest info
    let info = format!(
        "image={}\nregistry={}\nrepository={}\ntag={}\nlayers={}\n",
        image_ref.display(),
        image_ref.registry,
        image_ref.repository(),
        image_ref.tag,
        manifest.layers.len()
    );
    fs::write(image_dir.join("image.conf"), info)
        .map_err(|e| ContainerError::Filesystem(format!("write image config: {}", e)))?;

    println!("[+] Image {} pulled successfully.", image_ref.display());
    println!("    Rootfs: {}", rootfs_dir.display());

    Ok(rootfs_dir)
}

/// Extract a layer (tar.gz or tar) into the rootfs directory.
fn extract_layer(layer_file: &Path, rootfs: &Path, media_type: &str) -> Result<()> {
    let data = fs::read(layer_file)
        .map_err(|e| ContainerError::Filesystem(format!("read layer: {}", e)))?;

    // Check if gzipped (magic bytes 0x1f 0x8b)
    let tar_data = if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
        gzip::decompress(&data)?
    } else {
        data
    };

    // Extract tar to rootfs
    extract_tar_from_memory(&tar_data, rootfs)
}

/// Extract a tar archive from memory into a directory.
fn extract_tar_from_memory(data: &[u8], dest: &Path) -> Result<()> {
    let mut pos = 0;

    while pos + 512 <= data.len() {
        let header = &data[pos..pos + 512];

        // Check for end-of-archive (two zero blocks)
        if header.iter().all(|b| *b == 0) {
            break;
        }

        // Parse header fields
        let name = parse_tar_name(header);
        let size = parse_tar_octal(&header[124..136]);
        let typeflag = header[156];

        if name.is_empty() {
            pos += 512;
            continue;
        }

        // Skip whiteout files (.wh.) — used in overlay layers
        let basename = name.rsplit('/').next().unwrap_or(&name);
        if basename.starts_with(".wh.") {
            // Whiteout: delete the corresponding file
            let target_name = basename.trim_start_matches(".wh.");
            let parent = Path::new(&name).parent().unwrap_or(Path::new(""));
            let whiteout_target = dest.join(parent).join(target_name);
            if whiteout_target.exists() {
                let _ = fs::remove_dir_all(&whiteout_target);
                let _ = fs::remove_file(&whiteout_target);
            }
            pos += 512;
            let data_blocks = (size as usize + 511) / 512;
            pos += data_blocks * 512;
            continue;
        }

        let full_path = dest.join(&name);

        match typeflag {
            b'5' | b'/' => {
                // Directory
                let _ = fs::create_dir_all(&full_path);
            }
            b'0' | b'\0' => {
                // Regular file
                if let Some(parent) = full_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let data_start = pos + 512;
                let data_end = data_start + size as usize;
                if data_end <= data.len() {
                    let _ = fs::write(&full_path, &data[data_start..data_end]);
                }
            }
            b'2' => {
                // Symbolic link
                let link_target = parse_tar_linkname(header);
                if let Some(parent) = full_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                // On Linux, create symlink
                #[cfg(target_os = "linux")]
                {
                    let _ = std::os::unix::fs::symlink(&link_target, &full_path);
                }
                // On Windows, just skip symlinks for now
            }
            b'1' => {
                // Hard link
                let link_target = parse_tar_linkname(header);
                let target_path = dest.join(&link_target);
                if target_path.exists() {
                    if let Some(parent) = full_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::hard_link(&target_path, &full_path);
                }
            }
            _ => {
                // Skip other types (long names, pax extensions, etc.)
            }
        }

        pos += 512;
        let data_blocks = (size as usize + 511) / 512;
        pos += data_blocks * 512;
    }

    Ok(())
}

fn parse_tar_name(header: &[u8]) -> String {
    let prefix = extract_tar_string(&header[345..500]);
    let name = extract_tar_string(&header[0..100]);

    if !prefix.is_empty() {
        format!("{}/{}", prefix, name)
    } else {
        name
    }
}

fn parse_tar_linkname(header: &[u8]) -> String {
    extract_tar_string(&header[157..257])
}

fn extract_tar_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).to_string()
}

fn parse_tar_octal(buf: &[u8]) -> u64 {
    let s = extract_tar_string(buf);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return 0;
    }
    u64::from_str_radix(trimmed, 8).unwrap_or(0)
}

// ─── Storage ────────────────────────────────────────────────────────────────

fn image_storage_dir(image_ref: &ImageRef) -> PathBuf {
    let base = crate::config::state_base_dir().join("images");
    let dir_name = format!("{}_{}", image_ref.name, image_ref.tag.replace(":", "_"));
    base.join(dir_name)
}

/// List all locally stored images.
pub fn list_local_images() -> Result<Vec<(String, PathBuf)>> {
    let base = crate::config::state_base_dir().join("images");
    if !base.exists() {
        return Ok(Vec::new());
    }

    let mut images = Vec::new();
    for entry in fs::read_dir(&base)? {
        let entry = entry?;
        if entry.path().is_dir() {
            let conf_path = entry.path().join("image.conf");
            if conf_path.exists() {
                let conf = fs::read_to_string(&conf_path).unwrap_or_default();
                let name = conf.lines()
                    .find(|l| l.starts_with("image="))
                    .map(|l| l[6..].to_string())
                    .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
                let rootfs = entry.path().join("rootfs");
                images.push((name, rootfs));
            }
        }
    }

    Ok(images)
}
