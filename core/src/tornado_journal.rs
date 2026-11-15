//! Recovery data only. Identifiers are opaque Shell item IDs, not display names.
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"SNOWTJ01";
const LIMIT: usize = 8 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct DesktopSnapshot {
    pub folder_flags: u32,
    pub view_options: u32,
    pub items: Vec<DesktopItem>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DesktopItem { pub id: Vec<u8>, pub x: i32, pub y: i32 }

fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(2_166_136_261u32, |hash, &byte| (hash ^ u32::from(byte)).wrapping_mul(16_777_619))
}

fn invalid() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "invalid desktop recovery journal") }

fn valid_pidl(id: &[u8]) -> bool {
    let mut at = 0usize;
    loop {
        let Some(header) = id.get(at..at.saturating_add(2)) else { return false };
        let size = u16::from_le_bytes([header[0], header[1]]) as usize;
        if size == 0 { return at + 2 == id.len(); }
        if size < 2 { return false; }
        let Some(next) = at.checked_add(size) else { return false };
        if next > id.len() { return false; }
        at = next;
    }
}

impl DesktopSnapshot {
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        if self.items.len() > 10_000 { return Err(invalid()); }
        let mut out = Vec::new();
        out.extend(MAGIC); out.extend(self.folder_flags.to_le_bytes()); out.extend(self.view_options.to_le_bytes());
        out.extend((self.items.len() as u32).to_le_bytes());
        for item in &self.items {
            if item.id.len() < 2 || item.id.len() > 65_536 || !valid_pidl(&item.id) { return Err(invalid()); }
            if out.len().saturating_add(item.id.len()).saturating_add(16) > LIMIT { return Err(invalid()); }
            out.extend((item.id.len() as u32).to_le_bytes()); out.extend(item.x.to_le_bytes()); out.extend(item.y.to_le_bytes());
            out.extend(&item.id);
        }
        let hash = checksum(&out); out.extend(hash.to_le_bytes()); Ok(out)
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        if data.len() < 24 || data.len() > LIMIT || &data[..8] != MAGIC { return Err(invalid()); }
        let payload = data.len() - 4;
        if checksum(&data[..payload]) != u32::from_le_bytes(data[payload..].try_into().map_err(|_| invalid())?) { return Err(invalid()); }
        let mut offset = 8usize;
        let read = |offset: &mut usize| -> io::Result<u32> {
            let end = offset.checked_add(4).ok_or_else(invalid)?;
            if end > payload { return Err(invalid()); }
            let n = u32::from_le_bytes(data[*offset..end].try_into().map_err(|_| invalid())?); *offset = end; Ok(n)
        };
        let folder_flags=read(&mut offset)?; let view_options=read(&mut offset)?; let count=read(&mut offset)? as usize;
        if count > 10_000 { return Err(invalid()); }
        let mut items=Vec::with_capacity(count);
        for _ in 0..count {
            let len=read(&mut offset)? as usize; let x=read(&mut offset)? as i32; let y=read(&mut offset)? as i32;
            let end=offset.checked_add(len).ok_or_else(invalid)?;
            if !(2..=65_536).contains(&len) || end > payload || !valid_pidl(&data[offset..end]) { return Err(invalid()); }
            items.push(DesktopItem { id: data[offset..end].to_vec(), x, y }); offset=end;
        }
        if offset != payload { return Err(invalid()); }
        Ok(Self { folder_flags, view_options, items })
    }
}

/// Never overwrite an existing recovery record. The effect must not start on failure.
pub fn save_new(path: &Path, snapshot: &DesktopSnapshot) -> io::Result<()> {
    let bytes=snapshot.encode()?;
    let mut file=OpenOptions::new().write(true).create_new(true).open(path)?;
    if let Err(error)=file.write_all(&bytes).and_then(|_| file.sync_all()) {
        drop(file); let _=fs::remove_file(path); return Err(error);
    }
    Ok(())
}

/// Keep the file after reading; only remove it after a verified successful restoration.
pub fn load(path: &Path) -> io::Result<DesktopSnapshot> {
    let file=fs::File::open(path)?; let mut bytes=Vec::new();
    file.take((LIMIT+1) as u64).read_to_end(&mut bytes)?;
    DesktopSnapshot::decode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_roundtrip_rejects_corruption_and_preserves_existing_snapshot() {
        let snapshot=DesktopSnapshot { folder_flags: 1, view_options: 2, items: vec![DesktopItem { id: vec![4,0,7,8,0,0], x:-1920, y:75 }] };
        let encoded=snapshot.encode().unwrap(); assert_eq!(DesktopSnapshot::decode(&encoded).unwrap(), snapshot);
        let mut damaged=encoded.clone(); damaged[12]^=1; assert!(DesktopSnapshot::decode(&damaged).is_err());
        assert!(DesktopSnapshot::decode(&encoded[..encoded.len()-1]).is_err());
        let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("snowhush-tornado-{}-{stamp}.snapshot", std::process::id()));
        save_new(&path, &snapshot).unwrap(); assert!(save_new(&path, &snapshot).is_err());
        assert_eq!(load(&path).unwrap(), snapshot); fs::remove_file(path).unwrap();
    }
}
