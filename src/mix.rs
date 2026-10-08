//! Mixage de sources audio PCM 16 bits (logique pure).
//!
//! Chaque source livre un flux continu (sa `Timeline` comble les trous) qui démarre à
//! la même origine QPC : l'échantillon k de chaque file tombe au même instant. Le
//! mixeur ne livre que ce que toutes les sources ont déjà fourni.
#![forbid(unsafe_code)]

use std::collections::VecDeque;

pub struct Mixer {
    sources: Vec<VecDeque<i16>>,
}

impl Mixer {
    pub fn new(sources: usize) -> Self {
        Self {
            sources: vec![VecDeque::new(); sources],
        }
    }

    pub fn push(&mut self, source: usize, samples: &[i16]) {
        self.sources[source].extend(samples);
    }

    pub fn push_silence(&mut self, source: usize, samples: usize) {
        let queue = &mut self.sources[source];
        queue.resize(queue.len() + samples, 0);
    }

    /// Somme (saturée) de la partie commune à toutes les sources, retirée des files.
    pub fn pull(&mut self) -> Vec<i16> {
        let n = self.sources.iter().map(VecDeque::len).min().unwrap_or(0);
        let mut out = vec![0i16; n];
        for queue in &mut self.sources {
            for (o, s) in out.iter_mut().zip(queue.drain(..n)) {
                *o = o.saturating_add(s);
            }
        }
        out
    }
}

/// Stéréo entrelacé → même signal mono sur les deux canaux (moyenne L/R), comme le
/// « Downmix to Mono » d'OBS. Pour les micros : une interface comme la Scarlett Solo
/// expose 2 canaux (micro à gauche, entrée instrument à droite), la voix ne sortirait
/// sinon que d'un côté.
pub fn downmix_to_mono(stereo: &mut [i16]) {
    for frame in stereo.as_chunks_mut::<2>().0 {
        let mono = ((i32::from(frame[0]) + i32::from(frame[1])) / 2) as i16;
        *frame = [mono, mono];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_on_left_only_reaches_both_ears() {
        let mut s = [1000, 0, -400, 0];
        downmix_to_mono(&mut s);
        assert_eq!(s, [500, 500, -200, -200]);
    }

    #[test]
    fn already_mono_signal_is_unchanged() {
        let mut s = [300, 300, -7, -7];
        downmix_to_mono(&mut s);
        assert_eq!(s, [300, 300, -7, -7]);
    }

    #[test]
    fn downmix_cannot_overflow() {
        let mut s = [i16::MAX, i16::MAX, i16::MIN, i16::MIN];
        downmix_to_mono(&mut s);
        assert_eq!(s, [i16::MAX, i16::MAX, i16::MIN, i16::MIN]);
    }

    #[test]
    fn single_source_passes_through() {
        let mut m = Mixer::new(1);
        m.push(0, &[1, -2, 3]);
        assert_eq!(m.pull(), vec![1, -2, 3]);
        assert!(m.pull().is_empty());
    }

    #[test]
    fn aligned_sources_are_summed() {
        let mut m = Mixer::new(2);
        m.push(0, &[100, 200]);
        m.push(1, &[10, -20]);
        assert_eq!(m.pull(), vec![110, 180]);
    }

    #[test]
    fn only_the_common_part_is_delivered() {
        let mut m = Mixer::new(2);
        m.push(0, &[1, 2, 3, 4]);
        m.push(1, &[10, 20]);
        assert_eq!(m.pull(), vec![11, 22]);
        // Le reste de la source 0 attend la source 1, au même index.
        m.push(1, &[30, 40, 50]);
        assert_eq!(m.pull(), vec![33, 44]);
        m.push(0, &[5]);
        assert_eq!(m.pull(), vec![55]);
    }

    #[test]
    fn late_source_blocks_until_it_catches_up() {
        let mut m = Mixer::new(2);
        m.push(0, &[1; 480]);
        assert!(m.pull().is_empty());
        m.push_silence(1, 480);
        assert_eq!(m.pull(), vec![1; 480]);
    }

    #[test]
    fn sum_saturates() {
        let mut m = Mixer::new(2);
        m.push(0, &[30_000, -30_000]);
        m.push(1, &[10_000, -10_000]);
        assert_eq!(m.pull(), vec![i16::MAX, i16::MIN]);
    }

    #[test]
    fn long_run_keeps_sources_aligned() {
        // 10 min de paquets de 10 ms (480 trames stéréo), tailles décalées entre sources.
        let mut m = Mixer::new(2);
        let mut total = 0;
        for k in 0..60_000 {
            m.push(0, &[1; 960]);
            if k % 2 == 1 {
                m.push(1, &[2; 1920]);
            }
            total += m.pull().iter().filter(|&&s| s == 3).count();
        }
        assert_eq!(total, 60_000 * 960);
    }
}
