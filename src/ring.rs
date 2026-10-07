//! Buffer circulaire de paquets encodés (logique pure).
//!
//! Éviction façon OBS (`replay_buffer_purge`) : on purge par l'avant jusqu'à une
//! keyframe, et seulement si la fenêtre restante couvre encore `keep`.
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

pub struct Ring {
    packets: VecDeque<Packet>,
    keep: i64,
}

impl Ring {
    /// `keep` : durée minimale conservée (durée du clip + 1 GOP).
    pub fn new(keep: i64) -> Self {
        Self {
            packets: VecDeque::new(),
            keep,
        }
    }

    pub fn push(&mut self, packet: Packet) {
        // Un clip doit commencer sur une keyframe : rien ne sert avant la première.
        if self.packets.is_empty() && !packet.key {
            return;
        }
        let newest = packet.ts;
        self.packets.push_back(packet);
        while let Some(next_key) = self.packets.iter().skip(1).position(|p| p.key) {
            if newest - self.packets[next_key + 1].ts < self.keep {
                break;
            }
            self.packets.drain(..=next_key);
        }
    }

    pub fn last_ts(&self) -> Option<i64> {
        self.packets.back().map(|p| p.ts)
    }

    /// Paquets d'un clip de `dur` se terminant à `end` : départ à la dernière keyframe
    /// de ts ≤ `end - dur` (la plus ancienne à défaut), arrivée au dernier paquet ≤ `end`.
    pub fn snapshot(&self, end: i64, dur: i64) -> Vec<Packet> {
        let start = self
            .packets
            .iter()
            .rposition(|p| p.key && p.ts <= end - dur)
            .unwrap_or(0);
        self.packets
            .iter()
            .skip(start)
            .take_while(|p| p.ts <= end)
            .cloned()
            .collect()
    }

    #[cfg(test)]
    fn timestamps(&self) -> Vec<i64> {
        self.packets.iter().map(|p| p.ts).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un paquet par unité de temps, keyframe tous les `gop`.
    fn filled(keep: i64, count: i64, gop: i64) -> Ring {
        let mut ring = Ring::new(keep);
        for ts in 0..count {
            ring.push(packet(ts, ts % gop == 0));
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
        ring.push(packet(0, false));
        ring.push(packet(1, false));
        ring.push(packet(2, true));
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
        assert!(ring.packets.front().is_some_and(|p| p.key));
    }

    #[test]
    fn memory_is_bounded() {
        let ring = filled(25, 100_000, 10);
        assert!(ring.packets.len() <= 25 + 10 + 1);
    }

    #[test]
    fn snapshot_starts_on_keyframe_at_or_before_requested_start() {
        let ring = filled(35, 100, 10);
        let clip = ring.snapshot(95, 30); // début voulu : 65 → keyframe 60
        assert_eq!(clip.first().map(|p| p.ts), Some(60));
        assert!(clip[0].key);
        assert_eq!(clip.last().map(|p| p.ts), Some(95));
    }

    #[test]
    fn snapshot_on_exact_keyframe() {
        let ring = filled(35, 100, 10);
        let clip = ring.snapshot(90, 30);
        assert_eq!(clip.first().map(|p| p.ts), Some(60));
    }

    #[test]
    fn snapshot_shorter_buffer_returns_everything_from_oldest_keyframe() {
        let ring = filled(35, 12, 10); // seulement 0..=11 en mémoire
        let clip = ring.snapshot(11, 30);
        assert_eq!(clip.first().map(|p| p.ts), Some(0));
        assert_eq!(clip.len(), 12);
    }

    #[test]
    fn snapshot_excludes_packets_after_end() {
        let ring = filled(35, 100, 10);
        let clip = ring.snapshot(80, 30);
        assert_eq!(clip.last().map(|p| p.ts), Some(80));
    }

    #[test]
    fn snapshot_of_empty_ring_is_empty() {
        assert!(Ring::new(10).snapshot(100, 30).is_empty());
    }
}
