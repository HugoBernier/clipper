//! Buffer circulaire de paquets encodés (logique pure).
//!
//! Éviction façon OBS (`replay_buffer_purge`) : on purge par l'avant jusqu'à une
//! keyframe, et seulement si la fenêtre restante couvre encore `keep`. L'audio suit :
//! tout paquet audio plus ancien que la première image est retiré.
#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::sync::Arc;

/// Un paquet encodé. `ts` en unités de 100 ns (horloge QPC).
#[derive(Clone, Debug)]
pub struct Packet {
    pub ts: i64,
    pub key: bool,
    pub data: Arc<[u8]>,
}

/// Les paquets d'un clip, vidéo et audio, triés par ts.
#[derive(Debug, Default)]
pub struct Clip {
    pub video: Vec<Packet>,
    pub audio: Vec<Packet>,
}

pub struct Ring {
    video: VecDeque<Packet>,
    audio: VecDeque<Packet>,
    keep: i64,
}

impl Ring {
    /// `keep` : durée minimale conservée (durée du clip + 1 GOP).
    pub fn new(keep: i64) -> Self {
        Self {
            video: VecDeque::new(),
            audio: VecDeque::new(),
            keep,
        }
    }

    pub fn push_video(&mut self, packet: Packet) {
        // Un clip doit commencer sur une keyframe : rien ne sert avant la première.
        if self.video.is_empty() && !packet.key {
            return;
        }
        let newest = packet.ts;
        self.video.push_back(packet);
        while let Some(next_key) = self.video.iter().skip(1).position(|p| p.key) {
            if newest - self.video[next_key + 1].ts < self.keep {
                break;
            }
            self.video.drain(..=next_key);
        }
        self.evict_audio();
    }

    pub fn push_audio(&mut self, packet: Packet) {
        self.audio.push_back(packet);
        self.evict_audio();
    }

    fn evict_audio(&mut self) {
        let Some(oldest) = self.video.front().map(|p| p.ts) else {
            return;
        };
        while self.audio.front().is_some_and(|a| a.ts < oldest) {
            self.audio.pop_front();
        }
    }

    /// ts de la dernière image : sert à savoir si l'encodeur a rattrapé un instant.
    pub fn last_ts(&self) -> Option<i64> {
        self.video.back().map(|p| p.ts)
    }

    /// ts de la dernière trame audio, même usage que `last_ts`.
    pub fn last_audio_ts(&self) -> Option<i64> {
        self.audio.back().map(|p| p.ts)
    }

    /// Clip de `dur` se terminant à `end` : la vidéo part de la dernière keyframe de
    /// ts ≤ `end - dur` (la plus ancienne à défaut) ; l'audio couvre le même intervalle.
    pub fn snapshot(&self, end: i64, dur: i64) -> Clip {
        let start = self
            .video
            .iter()
            .rposition(|p| p.key && p.ts <= end - dur)
            .unwrap_or(0);
        let video: Vec<Packet> = self
            .video
            .iter()
            .skip(start)
            .take_while(|p| p.ts <= end)
            .cloned()
            .collect();
        let Some(first) = video.first().map(|p| p.ts) else {
            return Clip::default();
        };
        let audio = self
            .audio
            .iter()
            .filter(|a| a.ts >= first && a.ts <= end)
            .cloned()
            .collect();
        Clip { video, audio }
    }

    #[cfg(test)]
    fn timestamps(&self) -> Vec<i64> {
        self.video.iter().map(|p| p.ts).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un paquet par unité de temps, keyframe tous les `gop`.
    fn filled(keep: i64, count: i64, gop: i64) -> Ring {
        let mut ring = Ring::new(keep);
        for ts in 0..count {
            ring.push_video(packet(ts, ts % gop == 0));
        }
        ring
    }

    fn packet(ts: i64, key: bool) -> Packet {
        Packet {
            ts,
            key,
            data: Arc::from(vec![0u8; 1]),
        }
    }

    #[test]
    fn ignores_packets_before_first_keyframe() {
        let mut ring = Ring::new(100);
        ring.push_video(packet(0, false));
        ring.push_video(packet(1, false));
        ring.push_video(packet(2, true));
        assert_eq!(ring.timestamps(), vec![2]);
    }

    #[test]
    fn evicts_whole_gops_while_keeping_at_least_keep() {
        // keep 25, GOP 10 : après 0..=49, la fenêtre doit commencer sur une keyframe
        // et couvrir ≥ 25 → départ à 20 (49 - 20 = 29 ≥ 25 ; 30 donnerait 19).
        let ring = filled(25, 50, 10);
        assert_eq!(ring.timestamps().first(), Some(&20));
        assert_eq!(ring.last_ts(), Some(49));
    }

    #[test]
    fn front_is_always_a_keyframe() {
        let ring = filled(25, 1000, 10);
        assert!(ring.video.front().is_some_and(|p| p.key));
    }

    #[test]
    fn memory_is_bounded() {
        let ring = filled(25, 100_000, 10);
        assert!(ring.video.len() <= 25 + 10 + 1);
    }

    #[test]
    fn snapshot_starts_on_keyframe_at_or_before_requested_start() {
        let ring = filled(35, 100, 10);
        let clip = ring.snapshot(95, 30).video; // début voulu : 65 → keyframe 60
        assert_eq!(clip.first().map(|p| p.ts), Some(60));
        assert!(clip[0].key);
        assert_eq!(clip.last().map(|p| p.ts), Some(95));
    }

    #[test]
    fn snapshot_on_exact_keyframe() {
        let ring = filled(35, 100, 10);
        let clip = ring.snapshot(90, 30).video;
        assert_eq!(clip.first().map(|p| p.ts), Some(60));
    }

    #[test]
    fn snapshot_shorter_buffer_returns_everything_from_oldest_keyframe() {
        let ring = filled(35, 12, 10); // seulement 0..=11 en mémoire
        let clip = ring.snapshot(11, 30).video;
        assert_eq!(clip.first().map(|p| p.ts), Some(0));
        assert_eq!(clip.len(), 12);
    }

    #[test]
    fn snapshot_excludes_packets_after_end() {
        let ring = filled(35, 100, 10);
        let clip = ring.snapshot(80, 30).video;
        assert_eq!(clip.last().map(|p| p.ts), Some(80));
    }

    #[test]
    fn snapshot_of_empty_ring_is_empty() {
        let clip = Ring::new(10).snapshot(100, 30);
        assert!(clip.video.is_empty() && clip.audio.is_empty());
    }

    /// Vidéo 0..count (GOP `gop`) et un paquet audio toutes les 2 unités, décalé de 1.
    fn with_audio(keep: i64, count: i64, gop: i64) -> Ring {
        let mut ring = Ring::new(keep);
        for ts in 0..count {
            ring.push_video(packet(ts, ts % gop == 0));
            if ts % 2 == 1 {
                ring.push_audio(packet(ts, false));
            }
        }
        ring
    }

    #[test]
    fn last_audio_ts_is_the_newest_audio_packet() {
        let ring = with_audio(25, 100, 10);
        assert_eq!(ring.last_audio_ts(), Some(99));
        assert_eq!(Ring::new(10).last_audio_ts(), None);
    }

    #[test]
    fn audio_older_than_first_image_is_evicted() {
        let ring = with_audio(25, 100, 10);
        let oldest_video = ring.video.front().map(|p| p.ts).unwrap();
        assert!(ring.audio.iter().all(|a| a.ts >= oldest_video));
        assert!(!ring.audio.is_empty());
    }

    #[test]
    fn snapshot_audio_covers_video_interval() {
        let ring = with_audio(35, 100, 10);
        let clip = ring.snapshot(95, 30); // vidéo 60..=95
        assert_eq!(clip.audio.first().map(|p| p.ts), Some(61));
        assert_eq!(clip.audio.last().map(|p| p.ts), Some(95));
    }

    #[test]
    fn audio_before_any_video_is_kept_until_video_starts() {
        // L'audio peut démarrer avant la première image : rien à purger tant qu'aucune
        // image n'existe, puis purge dès la première keyframe.
        let mut ring = Ring::new(100);
        ring.push_audio(packet(0, false));
        ring.push_video(packet(5, true));
        assert!(ring.audio.is_empty());
    }
}
