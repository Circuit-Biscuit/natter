//! "Strong" noise suppression for calls: RNNoise (Xiph's recurrent network, via its Rust port nnnoiseless).
//!
//! Built on its own to `natter_denoise.wasm` (no wasm-bindgen, no imports) because it runs in the browser's
//! audio thread, an AudioWorklet (`crates/app/denoise.js`), where the app's own module can't go. The worklet
//! writes 480 samples (10 ms at 48 kHz, mono, scaled to the i16 range) to [`dn_input`], calls [`dn_process`]
//! and reads the cleaned frame from [`dn_output`].

use nnnoiseless::DenoiseState;

/// Samples per frame: 10 ms at 48 kHz.
pub const FRAME: usize = DenoiseState::FRAME_SIZE;

pub struct Denoiser {
    state: Box<DenoiseState<'static>>,
    input: [f32; FRAME],
    output: [f32; FRAME],
}

/// A new denoiser (one per microphone). Never freed: it lives as long as its worklet.
#[unsafe(no_mangle)]
pub extern "C" fn dn_new() -> *mut Denoiser {
    Box::into_raw(Box::new(Denoiser {
        state: DenoiseState::new(),
        input: [0.0; FRAME],
        output: [0.0; FRAME],
    }))
}

/// Where the worklet writes the next frame.
///
/// # Safety
/// `d` comes from [`dn_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dn_input(d: *mut Denoiser) -> *mut f32 {
    unsafe { (*d).input.as_mut_ptr() }
}

/// Where the worklet reads the cleaned frame.
///
/// # Safety
/// `d` comes from [`dn_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dn_output(d: *mut Denoiser) -> *mut f32 {
    unsafe { (*d).output.as_mut_ptr() }
}

/// Cleans the frame in the input buffer into the output buffer; returns how likely it holds speech (0–1).
///
/// # Safety
/// `d` comes from [`dn_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dn_process(d: *mut Denoiser) -> f32 {
    let d = unsafe { &mut *d };
    d.state.process_frame(&mut d.output, &d.input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// Fan-like noise (no speech) comes out much quieter, through the same calls the worklet makes. (Pure white
    /// noise barely changes: the network learned real rooms, and those don't sound like that.)
    #[test]
    fn removes_noise() {
        let d = dn_new();
        let (mut seed, mut low) = (1u32, 0.0f32);
        let (mut noise_in, mut noise_out) = (Vec::new(), Vec::new());
        for frame in 0..200 {
            let input: Vec<f32> = (0..FRAME)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    low = low * 0.95 + ((seed >> 16) as f32 / 32768.0 - 1.0) * 900.0;
                    low
                })
                .collect();
            unsafe {
                std::slice::from_raw_parts_mut(dn_input(d), FRAME).copy_from_slice(&input);
                dn_process(d);
                // The network needs a moment to learn the noise.
                if frame >= 100 {
                    noise_in.extend_from_slice(&input);
                    noise_out.extend_from_slice(std::slice::from_raw_parts(dn_output(d), FRAME));
                }
            }
        }
        let (before, after) = (rms(&noise_in), rms(&noise_out));
        assert!(
            after < before / 10.0,
            "noise only went from {before} to {after}"
        );
    }
}
