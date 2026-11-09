//! Downloads an on-device speech model one time, with progress and a SHA-256 check.

use crate::settings::LocalModel;
use crate::types::{MorselError, Result};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

const PARAKEET_REPO: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8";
const WHISPER_REPO: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-turbo/resolve/2ca6ff69fc878651b770880507669577ac41c2ff";

#[derive(Debug, Clone, PartialEq)]
pub struct ModelFile {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

impl LocalModel {
    /// The folder name of the model inside `<app data>/models/`.
    pub fn dir_name(self) -> &'static str {
        match self {
            LocalModel::Parakeet => "parakeet-tdt-0.6b-v3-int8",
            LocalModel::WhisperTurbo => "whisper-turbo-int8",
        }
    }

    /// The files of the model, pinned to one repository commit and one SHA-256 each.
    pub fn files(self) -> Vec<ModelFile> {
        let (repo, files): (&str, &[(&str, &str, u64)]) = match self {
            LocalModel::Parakeet => (
                PARAKEET_REPO,
                &[
                    (
                        "encoder.int8.onnx",
                        "1afbe69c1ffdabee94755f42e4daf1d93549131a3da11356bda84fcef0f2c479",
                        652_282_294,
                    ),
                    (
                        "decoder.int8.onnx",
                        "0222a86147299d4dee90edf95b6d13827edb42ad8b7c9cf0b2582f030937c0b5",
                        11_845_274,
                    ),
                    (
                        "joiner.int8.onnx",
                        "cbd02f3696e9944970111fb439d9c3fd365a80374565f13f2b9c7007d253fc98",
                        6_355_277,
                    ),
                    (
                        "tokens.txt",
                        "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
                        93_939,
                    ),
                ],
            ),
            LocalModel::WhisperTurbo => (
                WHISPER_REPO,
                &[
                    (
                        "turbo-encoder.int8.onnx",
                        "b02dcdf54f348741e93fe732b67d933c8dcb6735655f710640143081db38878b",
                        674_716_297,
                    ),
                    (
                        "turbo-decoder.int8.onnx",
                        "20accd02388482eb3a46bd615631adfdc85e1eb2c7db9ea3f02a40ffe6b81547",
                        361_080_764,
                    ),
                    (
                        "turbo-tokens.txt",
                        "b34b360dbb493e781e479794586d661700670d65564001f23024971d1f2fa126",
                        816_730,
                    ),
                ],
            ),
        };
        files
            .iter()
            .map(|&(name, sha, size)| ModelFile {
                name: name.into(),
                url: format!("{repo}/{name}"),
                sha256: sha.into(),
                size,
            })
            .collect()
    }
}

/// The folder that holds every on-device model.
pub fn models_root(app_data: &Path) -> PathBuf {
    app_data.join("models")
}

impl LocalModel {
    /// The folder of this model inside `models_root`.
    pub fn dir_in(self, root: &Path) -> PathBuf {
        root.join(self.dir_name())
    }
}

/// True when every file of the model is in `dir` with the right size.
pub fn model_ready(dir: &Path, model: LocalModel) -> bool {
    model.files().iter().all(|f| file_ready(dir, f))
}

fn file_ready(dir: &Path, file: &ModelFile) -> bool {
    std::fs::metadata(dir.join(&file.name))
        .map(|m| m.len() == file.size)
        .unwrap_or(false)
}

/// Downloads the files of the model that are missing. Returns the model folder.
pub async fn ensure_model(
    http: &reqwest::Client,
    dir: &Path,
    model: LocalModel,
    on_progress: impl Fn(u64, u64),
) -> Result<PathBuf> {
    ensure_files(http, dir, &model.files(), on_progress).await?;
    Ok(dir.to_path_buf())
}

/// Downloads each missing file to `<name>.part`, checks its SHA-256, then renames it.
pub async fn ensure_files(
    http: &reqwest::Client,
    dir: &Path,
    files: &[ModelFile],
    on_progress: impl Fn(u64, u64),
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let total: u64 = files.iter().map(|f| f.size).sum();
    let mut received: u64 = files
        .iter()
        .filter(|f| file_ready(dir, f))
        .map(|f| f.size)
        .sum();
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
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| MorselError::Http(e.to_string()))?
    {
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
            sha256: sha
                .map(str::to_owned)
                .unwrap_or_else(|| hex::encode(Sha256::digest(body))),
            size: body.len() as u64,
        }
    }

    #[tokio::test]
    async fn downloads_files_and_reports_progress() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a.onnx"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![7u8; 3000]))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file(&server, "a.onnx", &[7u8; 3000], None)];
        let last = std::sync::Mutex::new((0u64, 0u64));
        ensure_files(&reqwest::Client::new(), dir.path(), &files, |r, t| {
            *last.lock().unwrap() = (r, t)
        })
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("a.onnx")).unwrap().len(),
            3000
        );
        assert_eq!(*last.lock().unwrap(), (3000, 3000));
        assert!(!dir.path().join("a.onnx.part").exists());
    }

    #[tokio::test]
    async fn wrong_checksum_deletes_the_part_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 100]))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file(&server, "a.onnx", &[1u8; 100], Some(&"0".repeat(64)))];
        let err = ensure_files(&reqwest::Client::new(), dir.path(), &files, |_, _| {})
            .await
            .unwrap_err();
        assert!(matches!(err, MorselError::ModelChecksum));
        assert!(!dir.path().join("a.onnx.part").exists());
        assert!(!dir.path().join("a.onnx").exists());
    }

    #[tokio::test]
    async fn existing_model_is_not_downloaded_again() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.onnx"), [5u8; 50]).unwrap();
        let files = vec![file(&server, "a.onnx", &[5u8; 50], None)];
        ensure_files(&reqwest::Client::new(), dir.path(), &files, |_, _| {})
            .await
            .unwrap();
    }

    #[test]
    fn parakeet_files_are_pinned() {
        let files = LocalModel::Parakeet.files();
        assert_eq!(files.len(), 4);
        assert!(
            files.iter().all(|f| f.sha256.len() == 64
                && f.url.contains("ed5d745623472f97cfc2e6c2916ee248798fc5a8"))
        );
    }

    #[test]
    fn whisper_files_are_pinned() {
        let files = LocalModel::WhisperTurbo.files();
        let names: Vec<_> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "turbo-encoder.int8.onnx",
                "turbo-decoder.int8.onnx",
                "turbo-tokens.txt"
            ]
        );
        for f in &files {
            assert!(
                f.url
                    .contains("/resolve/2ca6ff69fc878651b770880507669577ac41c2ff/"),
                "{}",
                f.url
            );
            assert_eq!(f.sha256.len(), 64);
        }
        let total: u64 = files.iter().map(|f| f.size).sum();
        assert_eq!(total, 674_716_297 + 361_080_764 + 816_730);
    }

    #[test]
    fn each_model_has_its_own_folder() {
        let root = models_root(Path::new("/data"));
        let parakeet = LocalModel::Parakeet.dir_in(&root);
        let whisper = LocalModel::WhisperTurbo.dir_in(&root);
        assert_eq!(
            parakeet,
            Path::new("/data/models/parakeet-tdt-0.6b-v3-int8")
        );
        assert_eq!(whisper, Path::new("/data/models/whisper-turbo-int8"));
    }
}
