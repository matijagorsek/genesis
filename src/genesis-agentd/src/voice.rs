//! Voice input: transcribe a WAV clip with whisper.cpp, on this machine.

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::process::Command;

pub fn whisper_cli() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("GENESIS_WHISPER_CLI") { return Some(PathBuf::from(p)); }
    for c in ["/usr/lib/genesis/whisper/bin/whisper-cli", "/usr/local/bin/whisper-cli", "/opt/homebrew/bin/whisper-cli"] {
        if std::path::Path::new(c).is_file() { return Some(PathBuf::from(c)); }
    }
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':').map(|d| PathBuf::from(d).join("whisper-cli")).find(|p| p.is_file())
}

pub fn whisper_model() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("GENESIS_WHISPER_MODEL") { return Some(PathBuf::from(p)); }
    let dir = std::env::var("GENESIS_MODELS_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/var/lib/genesis/models")).join("stt");
    let mut c: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "bin").unwrap_or(false)).collect();
    // prefer the most capable model on disk (largest file): small over base over tiny
    c.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)));
    c.into_iter().next()
}

pub fn available() -> bool { whisper_cli().is_some() && whisper_model().is_some() }

/// `wav` must be 16 kHz mono PCM16 (the workspace records it that way).
pub fn transcribe(wav: &[u8]) -> Result<String> {
    let cli = whisper_cli().ok_or_else(|| anyhow!("whisper-cli is not installed"))?;
    let model = whisper_model().ok_or_else(|| anyhow!("no voice model under the models directory (stt/*.bin); the first-run wizard downloads one"))?;
    transcribe_with(&cli, &model, wav)
}

pub fn transcribe_with(cli: &std::path::Path, model: &std::path::Path, wav: &[u8]) -> Result<String> {
    let tmp = std::env::temp_dir().join(format!("genesis-voice-{}.wav", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, wav)?;
    let out = Command::new(&cli).args(["-m"]).arg(&model).args(["-f"]).arg(&tmp).args(["-nt", "-np", "-l", "auto", "-t", "4"]).output();
    let _ = std::fs::remove_file(&tmp);
    let out = out.map_err(|e| anyhow!("running {}: {}", cli.display(), e))?;
    if !out.status.success() {
        return Err(anyhow!("whisper failed: {}", String::from_utf8_lossy(&out.stderr).chars().take(400).collect::<String>()));
    }
    let text = String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real whisper.cpp on a spoken clip. Runs only when GENESIS_WHISPER_CLI, GENESIS_WHISPER_MODEL and
    /// GENESIS_VOICE_TEST_WAV are set (dev machines with whisper installed); CI covers the build.
    #[test]
    fn transcribes_a_clip_when_whisper_is_present() {
        let Ok(wav) = std::env::var("GENESIS_VOICE_TEST_WAV") else { eprintln!("no clip; skipping"); return; };
        if !available() { eprintln!("whisper not available; skipping"); return; }
        let text = transcribe(&std::fs::read(wav).unwrap()).unwrap().to_lowercase();
        eprintln!("transcript: {}", text);
        assert!(text.contains("pomodoro"), "{}", text);
    }

    #[test]
    fn missing_tools_are_reported() {
        let e = transcribe_with(std::path::Path::new("/nonexistent/whisper-cli"), std::path::Path::new("/nonexistent/model.bin"), b"RIFF").unwrap_err().to_string();
        assert!(e.contains("running"), "{}", e);
    }
}

// ---- voice output: Piper text-to-speech, on this machine ----------------------------------------

pub fn piper_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("GENESIS_PIPER") { return Some(PathBuf::from(p)); }
    for c in ["/usr/lib/genesis/piper/piper", "/usr/local/bin/piper", "/opt/homebrew/bin/piper"] {
        if std::path::Path::new(c).is_file() { return Some(PathBuf::from(c)); }
    }
    None
}

pub fn tts_model() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("GENESIS_TTS_MODEL") { return Some(PathBuf::from(p)); }
    let dir = std::env::var("GENESIS_MODELS_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/var/lib/genesis/models")).join("tts");
    let mut c: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "onnx").unwrap_or(false)).collect();
    c.sort();
    c.into_iter().next()
}

pub fn speech_available() -> bool { piper_bin().is_some() && tts_model().is_some() }

/// Returns a WAV clip of `text` spoken by the local Piper voice.
pub fn speak(text: &str) -> Result<Vec<u8>> {
    let bin = piper_bin().ok_or_else(|| anyhow!("piper is not installed"))?;
    let model = tts_model().ok_or_else(|| anyhow!("no voice under the models directory (tts/*.onnx); the first-run wizard downloads one"))?;
    speak_with(&bin, &model, text)
}

pub fn speak_with(bin: &std::path::Path, model: &std::path::Path, text: &str) -> Result<Vec<u8>> {
    use std::io::Write;
    let text: String = text.chars().take(4000).collect();
    if text.trim().is_empty() { return Err(anyhow!("nothing to say")); }
    let out = std::env::temp_dir().join(format!("genesis-say-{}.wav", uuid::Uuid::new_v4()));
    // the upstream build ships its libraries (espeak-ng, onnxruntime) next to the binary
    let libdir = bin.parent().map(|d| d.display().to_string()).unwrap_or_default();
    let mut child = Command::new(bin).args(["--model"]).arg(model).args(["--output_file"]).arg(&out)
        .env("LD_LIBRARY_PATH", &libdir).env("DYLD_LIBRARY_PATH", &libdir)
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped())
        .spawn().map_err(|e| anyhow!("running {}: {}", bin.display(), e))?;
    child.stdin.take().unwrap().write_all(text.as_bytes())?;
    let res = child.wait_with_output()?;
    let wav = std::fs::read(&out);
    let _ = std::fs::remove_file(&out);
    if !res.status.success() {
        return Err(anyhow!("piper failed: {}", String::from_utf8_lossy(&res.stderr).chars().take(400).collect::<String>()));
    }
    Ok(wav?)
}

#[cfg(test)]
mod tts_tests {
    use super::*;

    #[test]
    fn speaks_when_piper_is_present() {
        if !speech_available() { eprintln!("piper not available; skipping"); return; }
        let wav = speak("Genesis is ready.").unwrap();
        assert!(wav.len() > 10_000 && &wav[..4] == b"RIFF", "{} bytes", wav.len());
    }

    #[test]
    fn missing_piper_is_reported() {
        let e = speak_with(std::path::Path::new("/nonexistent/piper"), std::path::Path::new("/nonexistent/v.onnx"), "hi").unwrap_err().to_string();
        assert!(e.contains("running"), "{}", e);
    }
}
