//! The level of a stream, for the "You" and "Others" bars.

/// Root mean square of the samples, clamped to 0.0..=1.0. Empty input gives 0.0.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt().min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_silence_is_zero() {
        assert_eq!(rms(&[0.0; 480]), 0.0);
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn rms_of_full_scale_sine_is_about_0_707() {
        let s: Vec<f32> = (0..1600).map(|i| (i as f32 * std::f32::consts::TAU / 160.0).sin()).collect();
        assert!((rms(&s) - 0.707).abs() < 0.01);
    }

    #[test]
    fn rms_is_clamped_to_one() {
        assert_eq!(rms(&[3.0, -3.0]), 1.0);
    }
}
