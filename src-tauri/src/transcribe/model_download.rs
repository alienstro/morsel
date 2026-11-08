//! Downloads the Parakeet model one time, with progress and a SHA-256 check.

use crate::types::{MorselError, Result};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The folder name of the model inside `<app data>/models/`.
pub const PARAKEET_DIR: &str = "parakeet-tdt-0.6b-v3-int8";
const REPO: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8";

#[derive(Debug, Clone, PartialEq)]
pub struct ModelFile {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

/// The four Parakeet files, pinned to one repository commit and one SHA-256 each.
pub fn parakeet_files() -> Vec<ModelFile> {
    [
        ("encoder.int8.onnx", "1afbe69c1ffdabee94755f42e4daf1d93549131a3da11356bda84fcef0f2c479", 652_282_294),
        ("decoder.int8.onnx", "0222a86147299d4dee90edf95b6d13827edb42ad8b7c9cf0b2582f030937c0b5", 11_845_274),
        ("joiner.int8.onnx", "cbd02f3696e9944970111fb439d9c3fd365a80374565f13f2b9c7007d253fc98", 6_355_277),
        ("tokens.txt", "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d", 93_939),
    ]
    .into_iter()
    .map(|(name, sha, size)| ModelFile { name: name.into(), url: format!("{REPO}/{name}"), sha256: sha.into(), size })
    .collect()
}

pub fn model_dir(app_data: &Path) -> PathBuf {
    app_data.join("models").join(PARAKEET_DIR)
}

/// True when every Parakeet file is in `dir` with the right size.
pub fn model_ready(dir: &Path) -> bool {
    parakeet_files().iter().all(|f| file_ready(dir, f))
}

fn file_ready(dir: &Path, file: &ModelFile) -> bool {
    std::fs::metadata(dir.join(&file.name)).map(|m| m.len() == file.size).unwrap_or(false)
}

/// Downloads the Parakeet files that are missing. Returns the model folder.
pub async fn ensure_model(http: &reqwest::Client, dir: &Path, on_progress: impl Fn(u64, u64)) -> Result<PathBuf> {
    ensure_files(http, dir, &parakeet_files(), on_progress).await?;
    Ok(dir.to_path_buf())
}

/// Downloads each missing file to `<name>.part`, checks its SHA-256, then renames it.
pub async fn ensure_files(http: &reqwest::Client, dir: &Path, files: &[ModelFile], on_progress: impl Fn(u64, u64)) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let total: u64 = files.iter().map(|f| f.size).sum();
    let mut received: u64 = files.iter().filter(|f| file_ready(dir, f)).map(|f| f.size).sum();
    on_progress(received, total);
    for file in files.iter().filter(|f| !file_ready(dir, f)) {
        let part = dir.join(format!("{}.part", file.name));
        let result = download_one(http, file, &part, &mut received, total, &on_progress).await;
        if result.is_err() {
            let _ = std::fs::remove_file(&part);
            return result;
        }
        std::fs::rename(&part, dir.join(&file.name))?;
    }
    Ok(())
}

async fn download_one(
    http: &reqwest::Client,
    file: &ModelFile,
    part: &Path,
    received: &mut u64,
    total: u64,
    on_progress: &impl Fn(u64, u64),
) -> Result<()> {
    let mut response = http
        .get(&file.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| MorselError::Http(e.without_url().to_string()))?;
    let mut out = std::fs::File::create(part)?;
    let mut hasher = Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| MorselError::Http(e.to_string()))? {
        hasher.update(&chunk);
        out.write_all(&chunk)?;
        *received += chunk.len() as u64;
        on_progress(*received, total);
    }
    out.flush()?;
    if hex::encode(hasher.finalize()) != file.sha256 {
        return Err(MorselError::ModelChecksum);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn file(server: &MockServer, name: &str, body: &[u8], sha: Option<&str>) -> ModelFile {
        ModelFile {
            name: name.into(),
            url: format!("{}/{name}", server.uri()),
            sha256: sha.map(str::to_owned).unwrap_or_else(|| hex::encode(Sha256::digest(body))),
            size: body.len() as u64,
        }
    }

    #[tokio::test]
    async fn downloads_files_and_reports_progress() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/a.onnx")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![7u8; 3000])).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file(&server, "a.onnx", &[7u8; 3000], None)];
        let last = std::sync::Mutex::new((0u64, 0u64));
        ensure_files(&reqwest::Client::new(), dir.path(), &files, |r, t| *last.lock().unwrap() = (r, t)).await.unwrap();
        assert_eq!(std::fs::read(dir.path().join("a.onnx")).unwrap().len(), 3000);
        assert_eq!(*last.lock().unwrap(), (3000, 3000));
        assert!(!dir.path().join("a.onnx.part").exists());
    }

    #[tokio::test]
    async fn wrong_checksum_deletes_the_part_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 100])).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file(&server, "a.onnx", &[1u8; 100], Some(&"0".repeat(64)))];
        let err = ensure_files(&reqwest::Client::new(), dir.path(), &files, |_, _| {}).await.unwrap_err();
        assert!(matches!(err, MorselError::ModelChecksum));
        assert!(!dir.path().join("a.onnx.part").exists());
        assert!(!dir.path().join("a.onnx").exists());
    }

    #[tokio::test]
    async fn existing_model_is_not_downloaded_again() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.onnx"), [5u8; 50]).unwrap();
        let files = vec![file(&server, "a.onnx", &[5u8; 50], None)];
        ensure_files(&reqwest::Client::new(), dir.path(), &files, |_, _| {}).await.unwrap();
    }

    #[test]
    fn parakeet_files_are_pinned() {
        let files = parakeet_files();
        assert_eq!(files.len(), 4);
        assert!(files.iter().all(|f| f.sha256.len() == 64 && f.url.contains("ed5d745623472f97cfc2e6c2916ee248798fc5a8")));
    }
}
