//! OCR with what is already on the machine: the pack's multimodal small model reads text off an image.
//! A scanned PDF is rendered page by page with pdftoppm (poppler, already in the image for pdftotext),
//! up to a few pages; each page goes through the vision model with a transcription prompt.

use anyhow::{anyhow, Result};
use std::path::Path;

const MAX_PAGES: u32 = 4;

pub fn read_text(endpoint: &str, path: &Path) -> Result<String> {
    let lower = path.display().to_string().to_lowercase();
    if lower.ends_with(".pdf") {
        let dir = tempfile_dir()?;
        let prefix = dir.join("page");
        let st = std::process::Command::new("pdftoppm").args(["-png", "-r", "110", "-f", "1", "-l", &MAX_PAGES.to_string(), &path.display().to_string(), &prefix.display().to_string()]).status().map_err(|e| anyhow!("pdftoppm: {}", e))?;
        if !st.success() { return Err(anyhow!("could not render {}", path.display())); }
        let mut pages: Vec<_> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map(|x| x == "png").unwrap_or(false)).collect();
        pages.sort();
        let mut out = String::new();
        for (i, page) in pages.iter().enumerate() {
            let t = transcribe(endpoint, page)?;
            if !t.trim().is_empty() { out.push_str(&format!("--- page {} ---\n{}\n", i + 1, t.trim())); }
        }
        let _ = std::fs::remove_dir_all(&dir);
        Ok(out)
    } else {
        transcribe(endpoint, path)
    }
}

fn tempfile_dir() -> Result<std::path::PathBuf> {
    let base = std::env::var("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join(format!("genesis-ocr-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// One image through the vision model with a transcription prompt.
pub fn transcribe(endpoint: &str, image: &Path) -> Result<String> {
    let bytes = std::fs::read(image).map_err(|e| anyhow!("{}: {}", image.display(), e))?;
    if bytes.len() > 24 << 20 { return Err(anyhow!("{}: too large for the vision model", image.display())); }
    let mime = match image.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() { Some("jpg") | Some("jpeg") => "image/jpeg", Some("webp") => "image/webp", _ => "image/png" };
    let b64 = crate::base64_encode(&bytes);
    let model = crate::served_model_for(endpoint, "auto", "vision");
    let body = serde_json::json!({
        "model": model, "temperature": 0.0, "max_tokens": 1500,
        "messages": [
            {"role": "system", "content": "You transcribe text from images exactly. Output only the text you can read, in reading order, keeping line breaks. No commentary. If there is no text, output nothing."},
            {"role": "user", "content": [
                {"type": "text", "text": "Transcribe all text in this image."},
                {"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", mime, b64)}}
            ]}
        ]
    });
    let resp = ureq::post(&format!("{}/chat/completions", endpoint.trim_end_matches('/'))).timeout(std::time::Duration::from_secs(300)).send_json(body).map_err(|e| anyhow!("vision model: {}", e))?;
    let v: serde_json::Value = resp.into_json()?;
    Ok(v.pointer("/choices/0/message/content").and_then(|c| c.as_str()).unwrap_or("").to_string())
}
