//! Replay parsing (.osr layout, verified on real lazer files in M01).
//! Layout: mode u8, version i32, beatmap md5 str, player str, replay md5 str,
//! n300 n100 n50 geki katu miss (u16 each), score i32, max combo u16,
//! perfect u8, mods i32, life graph str, timestamp i64 (.NET ticks),
//! compressed data length i32, then LZMA data, then (version >= 20140721) an
//! online score id i64 and (version >= 30000001) a length-prefixed lazer score
//! blob (JSON, possibly compressed) holding the exact lazer mods.
//! str = 0x00 (none) or 0x0b + ULEB128 length + UTF-8.
//! M01 reads the header only; M02 decodes frames and the lazer mod list.

use std::io::{Cursor, Read};

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

pub const MOD_EASY: i32 = 1 << 1;
pub const MOD_HARDROCK: i32 = 1 << 4;
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

fn read_header(r: &mut Reader) -> Option<ReplayHeader> {
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

pub fn parse_header(bytes: &[u8]) -> Option<ReplayHeader> {
    read_header(&mut Reader { b: bytes, p: 0 })
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
/// JSON (see `LazerMod::speed_change`).
pub fn rate_from_mods(mods: i32) -> f64 {
    if mods & (MOD_DT | MOD_NC) != 0 {
        1.5
    } else if mods & MOD_HT != 0 {
        0.75
    } else {
        1.0
    }
}

// ---------------------------------------------------------------------------
// M02: frames and lazer mod list
// ---------------------------------------------------------------------------

/// One cursor/key sample. `t` is absolute beatmap time in ms (not real time).
/// `keys` is the legacy bit set: M1=1, M2=2, K1=4, K2=8, smoke=16.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub t: f64,
    pub x: f32,
    pub y: f32,
    pub keys: i32,
}

/// A mod from the lazer score blob. Only the settings the simulator needs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LazerMod {
    pub acronym: String,
    pub speed_change: Option<f64>,
    pub circle_size: Option<f64>,
    pub approach_rate: Option<f64>,
    pub overall_difficulty: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayData {
    pub header: ReplayHeader,
    pub frames: Vec<Frame>,
    /// None when the file has no readable lazer blob (legacy mod bits only).
    pub lazer_mods: Option<Vec<LazerMod>>,
}

/// Parse "w|x|y|keys," text. w is a delta; absolute time is the running sum.
/// Skips the -12345 seed frame, the two stable placeholder frames at the start
/// (x=256, y=-500) and any frame whose time goes backwards.
pub fn parse_frames_text(s: &str) -> Vec<Frame> {
    let mut out: Vec<Frame> = Vec::new();
    let mut t = 0f64;
    for (i, item) in s.split(',').enumerate() {
        let mut it = item.split('|');
        let (Some(w), Some(x), Some(y), Some(k)) = (it.next(), it.next(), it.next(), it.next())
        else {
            continue;
        };
        if w == "-12345" {
            continue;
        }
        let (Ok(w), Ok(x), Ok(y), Ok(k)) =
            (w.parse::<f64>(), x.parse::<f32>(), y.parse::<f32>(), k.parse::<i32>())
        else {
            continue;
        };
        t += w;
        if i < 2 && x == 256.0 && y == -500.0 {
            continue;
        }
        if let Some(last) = out.last() {
            if t < last.t {
                continue;
            }
        }
        out.push(Frame { t, x, y, keys: k });
    }
    out
}

pub fn decode_frames(compressed: &[u8]) -> Result<Vec<Frame>, String> {
    let mut input = Cursor::new(compressed);
    let mut out = Vec::new();
    lzma_rs::lzma_decompress(&mut input, &mut out).map_err(|e| format!("lzma: {e}"))?;
    Ok(parse_frames_text(&String::from_utf8_lossy(&out)))
}

/// The blob is JSON; try LZMA (real files), gzip, zlib, then plain text.
fn inflate_blob(blob: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    if blob.first() == Some(&0x5d) {
        // LZMA-alone (what real lazer files use, verified 2026-10-26)
        if lzma_rs::lzma_decompress(&mut Cursor::new(blob), &mut out).is_ok() {
            return Some(out);
        }
        out.clear();
    }
    if blob.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(blob).read_to_end(&mut out).ok()?;
        return Some(out);
    }
    if blob.first() == Some(&0x78) {
        if flate2::read::ZlibDecoder::new(blob).read_to_end(&mut out).is_ok() {
            return Some(out);
        }
        out.clear();
    }
    if blob.first() == Some(&b'{') {
        return Some(blob.to_vec());
    }
    None
}

pub fn parse_lazer_mods(json: &[u8]) -> Option<Vec<LazerMod>> {
    let v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let arr = v.get("mods")?.as_array()?;
    let mut out = Vec::new();
    for m in arr {
        let acronym = m.get("acronym")?.as_str()?.to_string();
        let s = m.get("settings");
        let num = |k: &str| s.and_then(|s| s.get(k)).and_then(|x| x.as_f64());
        out.push(LazerMod {
            acronym,
            speed_change: num("speed_change"),
            circle_size: num("circle_size"),
            approach_rate: num("approach_rate"),
            overall_difficulty: num("overall_difficulty"),
        });
    }
    Some(out)
}

fn read_trailer(r: &mut Reader, version: i32) -> Option<Vec<LazerMod>> {
    if version >= 20_140_721 {
        r.i64()?;
    } else if version >= 20_121_008 {
        r.i32()?;
    }
    if version < 30_000_001 {
        return None;
    }
    let n = r.i32()?;
    if n <= 0 {
        return None;
    }
    let blob = r.take(n as usize)?;
    parse_lazer_mods(&inflate_blob(blob)?)
}

/// Full parse: header, frames, and the lazer mod list when readable.
pub fn parse_full(bytes: &[u8]) -> Result<ReplayData, String> {
    let mut r = Reader { b: bytes, p: 0 };
    let header = read_header(&mut r).ok_or("bad header")?;
    let len = r.i32().ok_or("no frame data length")?;
    if len <= 0 {
        return Err("no frame data".into());
    }
    let comp = r.take(len as usize).ok_or("frame data truncated")?;
    let frames = decode_frames(comp)?;
    let lazer_mods = read_trailer(&mut r, header.version);
    Ok(ReplayData { header, frames, lazer_mods })
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

    fn head(v: &mut Vec<u8>, mode: u8, md5: &str, player: &str, mods: i32, counts: [u16; 4], ticks: i64) {
        v.push(mode);
        v.extend_from_slice(&30_000_018i32.to_le_bytes());
        put_str(v, md5);
        put_str(v, player);
        put_str(v, "d18b8fbc50675d3a3d85ab487f2c31bd");
        for c in [counts[0], counts[1], counts[2], 0, 0, counts[3]] {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v.extend_from_slice(&418_135i32.to_le_bytes());
        v.extend_from_slice(&245u16.to_le_bytes());
        v.push(0);
        v.extend_from_slice(&mods.to_le_bytes());
        put_str(v, "");
        v.extend_from_slice(&ticks.to_le_bytes());
    }

    pub fn build(mode: u8, md5: &str, player: &str, mods: i32, counts: [u16; 4], ticks: i64) -> Vec<u8> {
        let mut v = Vec::new();
        head(&mut v, mode, md5, player, mods, counts, ticks);
        v.extend_from_slice(&1000i32.to_le_bytes());
        v.extend_from_slice(&[0x5d, 0, 0, 0x20, 0]);
        v
    }

    /// Complete replay with real LZMA frames. `known_size` writes the real
    /// unpacked size into the LZMA header (as lazer does) instead of -1.
    pub fn build_full(
        md5: &str,
        player: &str,
        mods: i32,
        counts: [u16; 4],
        frames_text: &str,
        known_size: bool,
        blob: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut v = Vec::new();
        head(&mut v, 0, md5, player, mods, counts, 0);
        let mut comp = Vec::new();
        lzma_rs::lzma_compress(&mut Cursor::new(frames_text.as_bytes()), &mut comp).unwrap();
        if known_size {
            comp[5..13].copy_from_slice(&(frames_text.len() as u64).to_le_bytes());
        }
        v.extend_from_slice(&(comp.len() as i32).to_le_bytes());
        v.extend_from_slice(&comp);
        v.extend_from_slice(&0i64.to_le_bytes());
        if let Some(b) = blob {
            v.extend_from_slice(&(b.len() as i32).to_le_bytes());
            v.extend_from_slice(b);
        }
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

    #[test]
    fn frames_text_skips_placeholders_and_seed() {
        let f = parse_frames_text("0|256|-500|0,-1|256|-500|0,16|100|100|0,16|110|100|1,-12345|0|0|32123,");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0], Frame { t: 15.0, x: 100.0, y: 100.0, keys: 0 });
        assert_eq!(f[1], Frame { t: 31.0, x: 110.0, y: 100.0, keys: 1 });
    }

    #[test]
    fn frames_text_drops_backwards_time() {
        let f = parse_frames_text("0|1|1|0,10|2|2|0,-5|3|3|0,10|4|4|0");
        assert_eq!(f.iter().map(|f| f.t).collect::<Vec<_>>(), vec![0.0, 10.0, 15.0]);
    }

    #[test]
    fn full_parse_roundtrip_unknown_and_known_size() {
        let text = "0|256|-500|0,-1|256|-500|0,100|50|60|0,20|50|60|5,20|51|61|0,-12345|0|0|1,";
        for known in [false, true] {
            let b = build_full(MD5, "fr01hy", 0, [1, 0, 0, 0], text, known, None);
            let d = parse_full(&b).unwrap();
            assert_eq!(d.header.n300, 1);
            assert_eq!(d.frames.len(), 3, "known_size={known}");
            assert_eq!(d.frames[1].keys, 5);
            assert_eq!(d.frames[1].t, 119.0);
            assert!(d.lazer_mods.is_none());
        }
    }

    #[test]
    fn lazer_blob_plain_and_gzip() {
        use std::io::Write;
        let json = br#"{"mods":[{"acronym":"DT","settings":{"speed_change":1.3}},{"acronym":"HD"}],"statistics":{}}"#;
        let text = "100|1|1|0";
        let b = build_full(MD5, "p", 0, [0, 0, 0, 0], text, true, Some(json));
        let m = parse_full(&b).unwrap().lazer_mods.unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].acronym, "DT");
        assert_eq!(m[0].speed_change, Some(1.3));
        assert_eq!(m[1].speed_change, None);

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(json).unwrap();
        let gz = gz.finish().unwrap();
        let b = build_full(MD5, "p", 0, [0, 0, 0, 0], text, true, Some(&gz));
        assert_eq!(parse_full(&b).unwrap().lazer_mods.unwrap()[0].speed_change, Some(1.3));

        let mut lz = Vec::new();
        lzma_rs::lzma_compress(&mut Cursor::new(&json[..]), &mut lz).unwrap();
        let b = build_full(MD5, "p", 0, [0, 0, 0, 0], text, true, Some(&lz));
        assert_eq!(parse_full(&b).unwrap().lazer_mods.unwrap()[0].speed_change, Some(1.3));
    }
}
