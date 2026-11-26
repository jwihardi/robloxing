use anyhow::{Context, Result, ensure};
use image::{Rgb, RgbImage};
use serde::Deserialize;
use std::{f32::consts::TAU, fs, io::Write, path::Path};

pub const HOSTED_MODEL: &str = "eleven_text_to_sound_v3";
const RATE: u32 = 44_100;
const MAX_SECONDS: f32 = 12.0;
const SOUND_ENDPOINT: &str =
    "https://api.elevenlabs.io/v1/sound-generation?output_format=pcm_24000";
const CLOUD_RATE: u32 = 24_000;

pub(super) async fn hosted(folder: &Path, brief: &str, previous: Option<&Path>) -> Result<String> {
    let key = std::env::var("ELEVENLABS_API_KEY").context(
        "Set ELEVENLABS_API_KEY and restart Workshop, or turn off ElevenLabs in Sound effects.",
    )?;
    let mut prompt = brief.to_owned();
    if let Some(file) = previous {
        let original: serde_json::Value = serde_json::from_slice(&fs::read(file)?)?;
        if let Some(before) = original["brief"].as_str() {
            prompt = format!("{before}\nRevision: {brief}");
        }
    }
    let samples = fetch_sound(folder, &prompt, &key, SOUND_ENDPOINT).await?;
    save_wave(
        folder,
        &samples,
        CLOUD_RATE,
        &serde_json::json!({"model":HOSTED_MODEL,"brief":prompt}),
    )
}

async fn fetch_sound(folder: &Path, prompt: &str, key: &str, endpoint: &str) -> Result<Vec<f32>> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;

    ensure!(
        !key.is_empty() && key.bytes().all(|b| b.is_ascii_graphic()),
        "Check the ElevenLabs API key."
    );
    let tmp = tempfile::tempdir_in(folder)?;
    fs::write(
        tmp.path().join("request.json"),
        serde_json::to_vec(&serde_json::json!({"text":prompt,"model_id":HOSTED_MODEL}))?,
    )?;
    #[cfg(windows)]
    let curl = std::path::PathBuf::from(
        std::env::var_os("SystemRoot").context("Missing Windows system directory")?,
    )
    .join("System32/curl.exe");
    #[cfg(not(windows))]
    let curl = "curl";
    let mut cmd = tokio::process::Command::new(curl);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let protocol = if cfg!(test) { "=http,https" } else { "=https" };
    let mut child = cmd
        .current_dir(tmp.path())
        .env_remove("ELEVENLABS_API_KEY")
        .args([
            "--disable",
            "--silent",
            "--proto",
            protocol,
            "--max-time",
            "120",
            "--max-filesize",
            "1536000",
            "--header",
            "@-",
            "--header",
            "Content-Type: application/json",
            "--data-binary",
            "@request.json",
            "--output",
            "audio.pcm",
            "--write-out",
            "%{http_code} %{content_type}",
            endpoint,
        ])
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Could not start curl for ElevenLabs sound generation")?;
    // Pass the credential through the pipe so it never appears in command lines or project files.
    let mut headers = child.stdin.take().context("Missing request pipe")?;
    headers
        .write_all(format!("xi-api-key: {key}\n").as_bytes())
        .await?;
    drop(headers);
    let output = child.wait_with_output().await?;
    let meta = std::str::from_utf8(&output.stdout)?.trim();
    let (status, mime) = meta.split_once(' ').unwrap_or((meta, ""));
    ensure!(
        output.status.success(),
        "Could not download the ElevenLabs audio. Check the connection before retrying."
    );
    ensure!(
        status == "200",
        "ElevenLabs returned HTTP {status}. Check your Sound Effects API permission, credits and request."
    );
    ensure!(
        mime.starts_with("audio/") || mime.starts_with("application/octet-stream"),
        "ElevenLabs returned a non-audio response."
    );
    let pcm = fs::read(tmp.path().join("audio.pcm"))?;
    ensure!(
        (2..=1_536_000).contains(&pcm.len()) && pcm.len().is_multiple_of(2),
        "ElevenLabs returned incomplete PCM audio."
    );
    Ok(pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| i16::from_le_bytes(*p) as f32 / 32768.0)
        .collect())
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Wave {
    Sine,
    Triangle,
    Pulse,
    Noise,
}

#[derive(Deserialize)]
struct Layer {
    wave: Wave,
    start: f32,
    duration: f32,
    from_hz: f32,
    to_hz: f32,
    gain: f32,
    attack: f32,
    release: f32,
}

#[derive(Deserialize)]
struct Patch {
    layers: Vec<Layer>,
}

fn synthesize(patch: &Patch) -> Result<Vec<f32>> {
    ensure!(
        (1..=16).contains(&patch.layers.len()),
        "Use 1–16 sound layers."
    );
    let mut mix = Vec::new();
    for (index, layer) in patch.layers.iter().enumerate() {
        ensure!(
            (0.0..=MAX_SECONDS).contains(&layer.start)
                && (0.01..=MAX_SECONDS).contains(&layer.duration)
                && layer.start + layer.duration <= MAX_SECONDS
                && (20.0..=16_000.0).contains(&layer.from_hz)
                && (20.0..=16_000.0).contains(&layer.to_hz)
                && (0.0..=1.0).contains(&layer.gain)
                && (0.0..=layer.duration).contains(&layer.attack)
                && (0.0..=layer.duration).contains(&layer.release),
            "Sound layer {} exceeds the synthesis range.",
            index + 1
        );
        let start = (layer.start * RATE as f32).round() as usize;
        let count = (layer.duration * RATE as f32).round() as usize;
        mix.resize(mix.len().max(start + count), 0.0f32);
        let mut phase = 0.0f32;
        let mut filtered = 0.0f32;
        let mut noise = index as u32 + 1;
        for (i, sample) in mix[start..start + count].iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            let freq = layer.from_hz + (layer.to_hz - layer.from_hz) * t / layer.duration;
            phase = (phase + freq / RATE as f32).fract();
            let value = match layer.wave {
                Wave::Sine => (phase * TAU).sin(),
                Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
                Wave::Pulse => {
                    if phase < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                Wave::Noise => {
                    noise = noise.wrapping_mul(1664525).wrapping_add(1013904223);
                    let white = noise as f32 * (2.0 / u32::MAX as f32) - 1.0;
                    filtered += (1.0 - (-TAU * freq / RATE as f32).exp()) * (white - filtered);
                    filtered
                }
            };
            let remaining = (count - 1 - i) as f32 / RATE as f32;
            let envelope = (t / layer.attack.max(0.002))
                .min(remaining / layer.release.max(0.002))
                .min(1.0);
            *sample += value * layer.gain * envelope;
        }
    }
    let peak = mix.iter().fold(0f32, |p, s| p.max(s.abs()));
    ensure!(
        peak > 0.0001,
        "The sound designer returned a silent effect."
    );
    let gain = 0.9 / peak.max(1.0);
    for sample in &mut mix {
        *sample *= gain;
    }
    Ok(mix)
}

pub(super) fn render(folder: &Path, brief: &str, design: &serde_json::Value) -> Result<String> {
    let samples = synthesize(&serde_json::from_value(design.clone())?)?;
    let mut recipe = design.clone();
    recipe["brief"] = brief.into();
    save_wave(folder, &samples, RATE, &recipe)
}

fn save_wave(
    folder: &Path,
    samples: &[f32],
    rate: u32,
    design: &serde_json::Value,
) -> Result<String> {
    let bytes = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + bytes as usize);
    wav.write_all(b"RIFF")?;
    wav.write_all(&(36 + bytes).to_le_bytes())?;
    wav.write_all(b"WAVEfmt ")?;
    wav.write_all(&16u32.to_le_bytes())?;
    wav.write_all(&1u16.to_le_bytes())?; // PCM
    wav.write_all(&1u16.to_le_bytes())?; // mono
    wav.write_all(&rate.to_le_bytes())?;
    wav.write_all(&(rate * 2).to_le_bytes())?;
    wav.write_all(&2u16.to_le_bytes())?;
    wav.write_all(&16u16.to_le_bytes())?;
    wav.write_all(b"data")?;
    wav.write_all(&bytes.to_le_bytes())?;
    for sample in samples {
        wav.extend_from_slice(&((sample * 32767.0).round() as i16).to_le_bytes());
    }
    fs::write(folder.join("sound.wav"), wav)?;
    fs::write(
        folder.join("sound.json"),
        serde_json::to_vec_pretty(design)?,
    )?;
    let mut preview = RgbImage::from_pixel(480, 80, Rgb([22, 31, 35]));
    for x in 0..480 {
        let range = x * samples.len() / 480..(x + 1) * samples.len() / 480;
        let height =
            (samples[range].iter().fold(0f32, |p, s| p.max(s.abs())) * 39.0).round() as u32;
        for y in (40 - height)..=(40 + height) {
            preview.put_pixel(x as u32, y, Rgb([126, 217, 191]));
        }
    }
    preview.save(folder.join("preview.png"))?;
    Ok(format!(
        "WAV · {:.2} s · {} kHz · mono",
        samples.len() as f32 / rate as f32,
        rate as f32 / 1000.0
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn hosted_request_sends_sound_model_and_decodes_pcm() -> Result<()> {
        use std::io::{BufRead, BufReader, Read};
        let server = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let url = format!(
            "http://{}/v1/sound-generation?output_format=pcm_24000",
            server.local_addr()?
        );
        let fixture = std::thread::spawn(move || -> Result<_> {
            let (mut socket, _) = server.accept()?;
            socket.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
            let mut reader = BufReader::new(&socket);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                ensure!(reader.read_line(&mut line)? > 0, "Truncated request");
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            let lower = headers.to_ascii_lowercase();
            let len: usize = lower
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .context("Missing length")?
                .parse()?;
            let mut body = vec![0; len];
            reader.read_exact(&mut body)?;
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/pcm\r\nContent-Length: 4\r\nConnection: close\r\n\r\n\x00\x40\x00\xc0")?;
            Ok((lower, serde_json::from_slice::<serde_json::Value>(&body)?))
        });
        let folder = tempfile::tempdir()?;
        let samples = fetch_sound(folder.path(), "A wooden door", "fixture-key", &url).await?;
        let (headers, body) = fixture.join().unwrap()?;
        assert!(headers.starts_with("post /v1/sound-generation?output_format=pcm_24000 "));
        assert!(headers.contains("xi-api-key: fixture-key"));
        assert_eq!(
            body,
            json!({"text":"A wooden door","model_id":"eleven_text_to_sound_v3"})
        );
        assert_eq!(samples, [0.5, -0.5]);
        assert_eq!(fs::read_dir(folder.path())?.count(), 0);
        Ok(())
    }

    #[test]
    fn sound_has_a_repeatable_waveform_and_valid_pcm_header() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut patch = json!({"layers":[{"wave":"noise","start":0.1,"duration":0.2,"from_hz":3000,"to_hz":40,"attack":0.01,"release":0.18,"gain":0.8}]});
        render(dir.path(), "Filtered noise", &patch)?;
        let wav = fs::read(dir.path().join("sound.wav"))?;
        assert_eq!(wav.len(), 44 + 13230 * 2);
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert!(wav[44..44 + 4410 * 2].iter().all(|b| *b == 0));
        assert!(wav[44..].iter().any(|b| *b != 0));
        assert_eq!(image::open(dir.path().join("preview.png"))?.width(), 480);
        render(dir.path(), "Filtered noise", &patch)?;
        assert_eq!(wav, fs::read(dir.path().join("sound.wav"))?);
        patch["layers"][0]["gain"] = json!(0);
        assert!(render(dir.path(), "Filtered noise", &patch).is_err());
        patch["layers"][0]["duration"] = json!(5000);
        assert!(render(dir.path(), "Filtered noise", &patch).is_err());
        Ok(())
    }
}
