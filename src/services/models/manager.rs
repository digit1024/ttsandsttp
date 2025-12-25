//! Model Manager
//!
//! Handles downloading, verification, and management of ML models.
//! Models are stored in the application data directory and automatically
//! downloaded on first use.

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use crate::domain::ModelType;

/// Model Manager
///
/// Manages the lifecycle of ML models: downloading, verification, and path resolution.
/// Models are stored in `{app_data_dir}/stttts/` and organized by type.
pub struct ModelManager {
    models_dir: PathBuf,
}

impl ModelManager {
    /// Create a new ModelManager
    /// Models will be stored in {app_data_dir}/stttts
    pub fn new() -> Result<Self> {
        let app_data_dir = dirs::data_dir()
            .context("Could not determine application data directory")?;
        let models_dir = app_data_dir.join("stttts");

        // Create models directory if it doesn't exist
        fs::create_dir_all(&models_dir)
            .context("Failed to create models directory")?;

        Ok(Self { models_dir })
    }

    /// Get the models directory path
    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    /// Get the path for a specific model type
    pub fn model_path(&self, model_type: &ModelType) -> PathBuf {
        self.models_dir.join(model_type.subdirectory())
    }

    /// Get the actual model directory (handles subdirectories from archives)
    pub fn actual_model_path(&self, model_type: &ModelType) -> PathBuf {
        let base_path = self.model_path(model_type);
        let subdir_path = base_path.join(model_type.default_model_name());
        
        // If subdirectory exists, use it; otherwise use base path
        if subdir_path.exists() {
            subdir_path
        } else {
            base_path
        }
    }

    /// Ensure models are present, downloading if necessary
    pub async fn ensure_models_present(&self, model_type: &ModelType) -> Result<PathBuf> {
        let model_path = self.model_path(model_type);
        let model_name = model_type.default_model_name();

        // Check if model directory exists and has required files
        if self.check_model_files(&model_path, model_type) {
            // Model already present - return silently (avoid duplicate messages)
            return Ok(model_path);
        }

        // For VAD, check if it's optional and skip if URL doesn't work
        if matches!(model_type, ModelType::Vad) {
            // Try to download, but don't fail if it doesn't work
            println!("📥 Model '{}' not found, attempting download...", model_name);
            if let Err(e) = self.download_model(model_type).await {
                println!("⚠️  VAD model download failed (optional): {}", e);
                println!("   STT will work without VAD, but pause detection may be less accurate.");
                // Return the path anyway - VAD is optional
                return Ok(model_path);
            }
        } else {
            println!("📥 Model '{}' not found, downloading...", model_name);
            self.download_model(model_type).await?;
        }

        // Verify after download
        if !self.check_model_files(&model_path, model_type) {
            // For VAD, this is okay
            if matches!(model_type, ModelType::Vad) {
                println!("⚠️  VAD model incomplete (optional)");
                return Ok(model_path);
            }
            anyhow::bail!(
                "Model '{}' download incomplete. Required files missing.",
                model_name
            );
        }

        println!("✅ Model '{}' ready", model_name);
        Ok(model_path)
    }

    /// Check if all required model files are present
    fn check_model_files(&self, model_path: &Path, model_type: &ModelType) -> bool {
        if !model_path.exists() {
            return false;
        }

        // Check if files are in a subdirectory (common with tar archives)
        let actual_path = if model_path.join(model_type.default_model_name()).exists() {
            model_path.join(model_type.default_model_name())
        } else {
            model_path.to_path_buf()
        };

        if !actual_path.exists() {
            return false;
        }

        let required_files = model_type.required_files();
        required_files.iter().all(|file| {
            let file_path = actual_path.join(file);
            file_path.exists() && file_path.is_file()
        })
    }

    /// Download and extract a model
    async fn download_model(&self, model_type: &ModelType) -> Result<()> {
        let url = model_type.default_url();
        let model_name = model_type.default_model_name();
        let model_path = self.model_path(model_type);

        // Create model directory
        fs::create_dir_all(&model_path)
            .context("Failed to create model directory")?;

        // Download the model
        println!("⬇️  Downloading from: {}", url);
        
        // Handle single .onnx files (like VAD) vs tar.bz2 archives
        if url.ends_with(".onnx") {
            // Single file download
            let filename = url.split('/').last().unwrap_or("model.onnx");
            let file_path = self.download_file(url, filename).await?;
            let dest_path = model_path.join(filename);
            tokio::fs::rename(&file_path, &dest_path)
                .await
                .context("Failed to move downloaded file")?;
        } else {
            // Archive download
            let archive_path = self.download_file(url, &format!("{}.tar.bz2", model_name)).await?;

            // Extract the archive
            println!("📦 Extracting model...");
            self.extract_tarbz2(&archive_path, &model_path).await?;

            // Clean up archive file
            fs::remove_file(&archive_path)
                .context("Failed to remove downloaded archive")?;
        }

        Ok(())
    }

    /// Download a file with progress bar
    async fn download_file(&self, url: &str, filename: &str) -> Result<PathBuf> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .user_agent("ttsandsttp/0.1.0")
            .build()
            .context("Failed to create HTTP client")?;
        
        let response = client
            .get(url)
            .send()
            .await
            .context(format!("Failed to download from: {}", url))?;

        // Check HTTP status
        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!(
                "Download failed with status {}: {}\nURL: {}",
                status,
                error_text,
                url
            );
        }

        let total_size = response
            .content_length()
            .context("Could not determine file size")?;
        
        // Verify minimum file size (at least 1KB)
        if total_size < 1024 {
            anyhow::bail!(
                "Downloaded file is too small ({} bytes). This might be an error page.\nURL: {}",
                total_size,
                url
            );
        }

        let pb = ProgressBar::new(total_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        pb.set_message(format!("Downloading {}", filename));

        let archive_path = self.models_dir.join(filename);
        let mut file = tokio::fs::File::create(&archive_path)
            .await
            .context("Failed to create archive file")?;

        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;

        use futures::StreamExt;
        while let Some(item) = stream.next().await {
            let chunk = item.context("Error while downloading")?;
            file.write_all(&chunk)
                .await
                .context("Error while writing to file")?;
            downloaded += chunk.len() as u64;
            pb.set_position(downloaded);
        }

        pb.finish_with_message(format!("Downloaded {}", filename));
        
        // Verify file size matches
        let file_size = tokio::fs::metadata(&archive_path)
            .await
            .context("Failed to get file metadata")?
            .len();
        
        if file_size != total_size {
            anyhow::bail!(
                "Download incomplete: expected {} bytes, got {} bytes",
                total_size,
                file_size
            );
        }
        
        if file_size < 1024 {
            anyhow::bail!(
                "Downloaded file is too small ({} bytes). This might be an error page.",
                file_size
            );
        }
        
        Ok(archive_path)
    }

    /// Extract a tar.bz2 archive
    async fn extract_tarbz2(&self, archive_path: &Path, extract_to: &Path) -> Result<()> {
        // Verify it's a bzip2 file by checking magic bytes
        let mut header = [0u8; 3];
        {
            let mut file = fs::File::open(archive_path)
                .context("Failed to open archive file")?;
            use std::io::Read;
            file.read_exact(&mut header)
                .context("Failed to read file header")?;
        }
        
        // BZ2 magic bytes: "BZ"
        if header[0] != b'B' || header[1] != b'Z' {
            anyhow::bail!(
                "File is not a valid bzip2 archive. Magic bytes: {:?}\nFile: {}",
                header,
                archive_path.display()
            );
        }

        let file = fs::File::open(archive_path)
            .context("Failed to open archive file")?;

        let decoder = bzip2::read::BzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);

        archive
            .unpack(extract_to)
            .context("Failed to extract archive")?;

        Ok(())
    }

    /// Get the path to a specific model file
    pub fn get_model_file(&self, model_type: &ModelType, filename: &str) -> PathBuf {
        self.actual_model_path(model_type).join(filename)
    }

    /// Download a model by URL and extract to a specific path
    /// This is used for config-based model downloads
    pub async fn download_model_by_url(
        &self,
        url: &str,
        model_id: &str,
        extract_to: &PathBuf,
        required_files: &[String],
    ) -> Result<()> {
        // Check if model is already downloaded
        let actual_path = self.find_actual_model_path(extract_to, model_id);
        let all_files_exist = required_files.iter().all(|file| {
            let file_path = actual_path.join(file);
            file_path.exists() && file_path.is_file()
        });

        if all_files_exist {
            return Ok(());
        }

        // Create model directory
        std::fs::create_dir_all(extract_to)
            .context("Failed to create model directory")?;

        // Download the model
        eprintln!("⬇️  Downloading from: {}", url);
        
        // Handle single .onnx files vs tar.bz2 archives
        if url.ends_with(".onnx") {
            // Single file download
            let filename = url.split('/').last().unwrap_or("model.onnx");
            let file_path = self.download_file(url, filename).await?;
            let dest_path = extract_to.join(filename);
            tokio::fs::rename(&file_path, &dest_path)
                .await
                .context("Failed to move downloaded file")?;
        } else {
            // Archive download
            let archive_path = self.download_file(url, &format!("{}.tar.bz2", model_id)).await?;

            // Extract the archive
            eprintln!("📦 Extracting model...");
            self.extract_tarbz2(&archive_path, extract_to).await?;

            // Clean up archive file
            std::fs::remove_file(&archive_path)
                .context("Failed to remove downloaded archive")?;
        }

        // Verify files exist (check both direct path and subdirectory)
        let actual_path = self.find_actual_model_path(extract_to, model_id);
        for file in required_files {
            let file_path = actual_path.join(file);
            if !file_path.exists() {
                anyhow::bail!(
                    "Required file not found after download: {} (checked in: {})",
                    file_path.display(),
                    actual_path.display()
                );
            }
        }

        Ok(())
    }

    /// Find the actual model path (handles subdirectories from archives)
    pub fn find_actual_model_path(&self, base_path: &Path, model_id: &str) -> PathBuf {
        let subdir_path = base_path.join(model_id);
        if subdir_path.exists() {
            subdir_path
        } else {
            base_path.to_path_buf()
        }
    }
}

impl Default for ModelManager {
    fn default() -> Self {
        Self::new().expect("Failed to create ModelManager")
    }
}
