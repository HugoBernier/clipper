//! Boîtes MP4 (ISO BMFF) : en-têtes et découpage (logique pure).
#![forbid(unsafe_code)]

use anyhow::{Context, Result, bail};

pub struct Mp4Box {
    pub kind: [u8; 4],
    pub start: usize,
    pub len: usize,
    pub header: usize,
}

impl Mp4Box {
    pub fn range(&self) -> std::ops::Range<usize> {
        self.start..self.start + self.len
    }
}

/// En-tête d'une boîte commençant à `head` : (taille totale, taille de l'en-tête, type).
/// Une taille 0 (« jusqu'à la fin ») est rendue telle quelle ; `None` si tronqué.
pub fn header(head: &[u8]) -> Option<(u64, usize, [u8; 4])> {
    let size = u32::from_be_bytes(head.get(..4)?.try_into().ok()?);
    let kind = head.get(4..8)?.try_into().ok()?;
    match size {
        1 => Some((
            u64::from_be_bytes(head.get(8..16)?.try_into().ok()?),
            16,
            kind,
        )),
        n => Some((u64::from(n), 8, kind)),
    }
}

/// Boîtes de `data[from..to]`, qui doivent y tenir entières.
pub fn parse(data: &[u8], from: usize, to: usize) -> Result<Vec<Mp4Box>> {
    let mut boxes = Vec::new();
    let mut i = from;
    while i < to {
        let head = data.get(i..to).context("en-tête de boîte tronqué")?;
        let (len, header, kind) = header(head).context("en-tête de boîte tronqué")?;
        let len = if len == 0 { to - i } else { len as usize };
        // Taille lue dans le fichier : un fichier corrompu ne doit pas faire déborder.
        if len < header || i.checked_add(len).is_none_or(|end| end > to) {
            bail!("boîte invalide à l'offset {i}");
        }
        boxes.push(Mp4Box {
            kind,
            start: i,
            len,
            header,
        });
        i += len;
    }
    Ok(boxes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_size_is_an_error_not_an_overflow() {
        // Une boîte de 8 octets, puis une boîte 64 bits dont la taille déborderait.
        let mut data = vec![0, 0, 0, 8, b'f', b'r', b'e', b'e'];
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(b"free");
        data.extend_from_slice(&u64::MAX.to_be_bytes());
        assert!(parse(&data, 0, data.len()).is_err());
    }
}
