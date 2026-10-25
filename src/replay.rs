//! Replay header parsing (.osr layout, verified on real lazer files in M01).
//! Layout: mode u8, version i32, beatmap md5 str, player str, replay md5 str,
//! n300 n100 n50 geki katu miss (u16 each), score i32, max combo u16,
//! perfect u8, mods i32, life graph str, timestamp i64 (.NET ticks),
//! compressed data length i32, then LZMA data (frames are not read in M01).
//! str = 0x00 (none) or 0x0b + ULEB128 length + UTF-8.

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayHeader {
    pub mode: u8,
    pub version: i32,
    pub beatmap_md5: String,
    pub player: String,
    pub mods: i32,
    pub n300: u16,
    pub n100: u16,
    pub n50: u16,
    pub nmiss: u16,
    pub played_at: i64, // unix seconds
}

pub const MOD_RELAX: i32 = 1 << 7;
pub const MOD_AUTOPILOT: i32 = 1 << 13;
pub const MOD_AUTO: i32 = 1 << 11;
pub const MOD_CINEMA: i32 = 1 << 22;
pub const MOD_DT: i32 = 1 << 6;
pub const MOD_NC: i32 = 1 << 9;
pub const MOD_HT: i32 = 1 << 8;

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.p.checked_add(n)?;
        let s = self.b.get(self.p..end)?;
        self.p = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn string(&mut self) -> Option<String> {
        match self.u8()? {
            0x00 => Some(String::new()),
            0x0b => {
                let mut len: usize = 0;
                let mut shift = 0;
                loop {
                    let byte = self.u8()?;
                    len |= ((byte & 0x7f) as usize) << shift;
                    if byte & 0x80 == 0 {
                        break;
                    }
                    shift += 7;
                    if shift > 28 {
                        return None;
                    }
                }
                let raw = self.take(len)?;
                Some(String::from_utf8_lossy(raw).into_owned())
            }
            _ => None,
        }
    }
}

const TICKS_AT_UNIX_EPOCH: i64 = 621_355_968_000_000_000;

pub fn ticks_to_unix(ticks: i64) -> i64 {
    (ticks - TICKS_AT_UNIX_EPOCH).div_euclid(10_000_000)
}

/// Cheap content test on the first bytes: mode 0..=3, plausible version, then
/// a string marker 0x0b followed by length 0x20 (an MD5 hex string).
pub fn looks_like_replay(prefix: &[u8]) -> bool {
    if prefix.len() < 7 || prefix[0] > 3 {
        return false;
    }
    let version = i32::from_le_bytes([prefix[1], prefix[2], prefix[3], prefix[4]]);
    version >= 20_000_000 && version < 40_000_000 && prefix[5] == 0x0b && prefix[6] == 0x20
}

pub fn parse_header(bytes: &[u8]) -> Option<ReplayHeader> {
    let mut r = Reader { b: bytes, p: 0 };
    let mode = r.u8()?;
    let version = r.i32()?;
    let beatmap_md5 = r.string()?;
    let player = r.string()?;
    let _replay_md5 = r.string()?;
    let n300 = r.u16()?;
    let n100 = r.u16()?;
    let n50 = r.u16()?;
    let _geki = r.u16()?;
    let _katu = r.u16()?;
    let nmiss = r.u16()?;
    let _score = r.i32()?;
    let _combo = r.u16()?;
    let _perfect = r.u8()?;
    let mods = r.i32()?;
    let _life = r.string()?;
    let ticks = r.i64()?;
    if beatmap_md5.len() != 32 {
        return None;
    }
    Some(ReplayHeader {
        mode,
        version,
        beatmap_md5,
        player,
        mods,
        n300,
        n100,
        n50,
        nmiss,
        played_at: ticks_to_unix(ticks),
    })
}

/// Reason a replay is excluded, or None if it is kept.
pub fn exclusion_reason(h: &ReplayHeader, own_player: &str) -> Option<&'static str> {
    if h.mode != 0 {
        return Some("not osu!standard");
    }
    if h.mods & MOD_CINEMA != 0 {
        return Some("cheat mod: Cinema");
    }
    if h.mods & MOD_AUTO != 0 {
        return Some("cheat mod: Auto");
    }
    if h.mods & MOD_RELAX != 0 {
        return Some("cheat mod: Relax");
    }
    if h.mods & MOD_AUTOPILOT != 0 {
        return Some("cheat mod: Autopilot");
    }
    if !own_player.is_empty() && !h.player.eq_ignore_ascii_case(own_player) {
        return Some("not own replay");
    }
    None
}

/// Rate from legacy mod bits only. Custom lazer rates live in the trailing
/// JSON and are read in M02.
pub fn rate_from_mods(mods: i32) -> f64 {
    if mods & (MOD_DT | MOD_NC) != 0 {
        1.5
    } else if mods & MOD_HT != 0 {
        0.75
    } else {
        1.0
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn put_str(v: &mut Vec<u8>, s: &str) {
        if s.is_empty() {
            v.push(0);
        } else {
            v.push(0x0b);
            let mut n = s.len();
            loop {
                let mut b = (n & 0x7f) as u8;
                n >>= 7;
                if n > 0 {
                    b |= 0x80;
                }
                v.push(b);
                if n == 0 {
                    break;
                }
            }
            v.extend_from_slice(s.as_bytes());
        }
    }

    pub fn build(mode: u8, md5: &str, player: &str, mods: i32, counts: [u16; 4], ticks: i64) -> Vec<u8> {
        let mut v = vec![mode];
        v.extend_from_slice(&30_000_018i32.to_le_bytes());
        put_str(&mut v, md5);
        put_str(&mut v, player);
        put_str(&mut v, "d18b8fbc50675d3a3d85ab487f2c31bd");
        for c in [counts[0], counts[1], counts[2], 0, 0, counts[3]] {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v.extend_from_slice(&418_135i32.to_le_bytes());
        v.extend_from_slice(&245u16.to_le_bytes());
        v.push(0);
        v.extend_from_slice(&mods.to_le_bytes());
        put_str(&mut v, "");
        v.extend_from_slice(&ticks.to_le_bytes());
        v.extend_from_slice(&1000i32.to_le_bytes());
        v.extend_from_slice(&[0x5d, 0, 0, 0x20, 0]);
        v
    }

    const MD5: &str = "3f8beee408d05f83de870f9f86fc375c";

    #[test]
    fn parses_header_fields() {
        let ticks = 621_355_968_000_000_000 + 1_700_000_000 * 10_000_000;
        let b = build(0, MD5, "fr01hy", 64, [857, 115, 4, 32], ticks);
        let h = parse_header(&b).unwrap();
        assert_eq!(h.mode, 0);
        assert_eq!(h.version, 30_000_018);
        assert_eq!(h.beatmap_md5, MD5);
        assert_eq!(h.player, "fr01hy");
        assert_eq!((h.n300, h.n100, h.n50, h.nmiss), (857, 115, 4, 32));
        assert_eq!(h.mods, 64);
        assert_eq!(h.played_at, 1_700_000_000);
    }

    #[test]
    fn matches_real_file_prefix() {
        // First 0x30 bytes of a real lazer replay (hexdump from the user's machine).
        let real: [u8; 16] = [
            0x00, 0x92, 0xc3, 0xc9, 0x01, 0x0b, 0x20, 0x33, 0x66, 0x38, 0x62, 0x65, 0x65, 0x65,
            0x34, 0x30,
        ];
        assert!(looks_like_replay(&real));
    }

    #[test]
    fn rejects_non_replay() {
        assert!(!looks_like_replay(b"osu file format v14\n"));
        assert!(!looks_like_replay(&[0u8; 3]));
        assert!(!looks_like_replay(&[9, 0, 0, 0, 0, 0x0b, 0x20]));
    }

    #[test]
    fn truncated_is_none() {
        let b = build(0, MD5, "fr01hy", 0, [1, 2, 3, 4], 0);
        assert!(parse_header(&b[..30]).is_none());
    }

    #[test]
    fn exclusions() {
        let mk = |mode, mods, name: &str| {
            parse_header(&build(mode, MD5, name, mods, [1, 0, 0, 0], 0)).unwrap()
        };
        assert_eq!(exclusion_reason(&mk(0, 0, "fr01hy"), "fr01hy"), None);
        assert_eq!(exclusion_reason(&mk(0, 0, "FR01HY"), "fr01hy"), None);
        assert_eq!(exclusion_reason(&mk(0, 0, "other"), "fr01hy"), Some("not own replay"));
        assert_eq!(exclusion_reason(&mk(0, MOD_RELAX, "fr01hy"), "fr01hy"), Some("cheat mod: Relax"));
        assert_eq!(exclusion_reason(&mk(0, MOD_AUTOPILOT, "fr01hy"), "fr01hy"), Some("cheat mod: Autopilot"));
        assert_eq!(exclusion_reason(&mk(0, MOD_AUTO, "fr01hy"), "fr01hy"), Some("cheat mod: Auto"));
        assert_eq!(exclusion_reason(&mk(0, MOD_CINEMA | MOD_AUTO, "fr01hy"), "fr01hy"), Some("cheat mod: Cinema"));
        assert_eq!(exclusion_reason(&mk(1, 0, "fr01hy"), "fr01hy"), Some("not osu!standard"));
    }

    #[test]
    fn rates() {
        assert_eq!(rate_from_mods(0), 1.0);
        assert_eq!(rate_from_mods(MOD_DT), 1.5);
        assert_eq!(rate_from_mods(MOD_NC | MOD_DT), 1.5);
        assert_eq!(rate_from_mods(MOD_HT), 0.75);
    }
}
