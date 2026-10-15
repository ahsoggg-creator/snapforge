//! A camera-shutter click built in code, so the "capture sound" option needs no audio file.

const RATE: u32 = 22_050;

/// Two quick clicks (a noise burst each, the second with a low thump), as a 16-bit mono WAV file in memory.
pub fn wav() -> Vec<u8> {
    let n = (RATE as f32 * 0.14) as usize;
    let mut seed: u32 = 0x1234_5678;
    let mut noise = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 16) as f32 / 32_768.0 - 1.0
    };
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        let click = |at: f32, len: f32| {
            let d = t - at;
            if (0.0..len).contains(&d) {
                (-d * 5.0 / len).exp()
            } else {
                0.0
            }
        };
        let thump = (t * 2.0 * std::f32::consts::PI * 170.0).sin() * click(0.06, 0.07);
        let v = noise() * (click(0.0, 0.012) * 0.7 + click(0.06, 0.016) * 0.9) + thump * 0.5;
        pcm.extend_from_slice(&((v.clamp(-1.0, 1.0) * 0.6 * f32::from(i16::MAX)) as i16).to_le_bytes());
    }
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(&pcm);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_click_is_a_well_formed_audible_wav() {
        let w = wav();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        let riff = u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize;
        let data = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(riff + 8, w.len(), "the RIFF size covers the whole file");
        assert_eq!(data + 44, w.len(), "the data size covers the samples");
        let peak = w[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs())
            .max()
            .unwrap();
        assert!(peak > 3_000, "not silent: {peak}");
        assert!(peak < 25_000, "not clipped: {peak}");
        assert!(w.len() < 10_000, "a short sound");
    }
}
