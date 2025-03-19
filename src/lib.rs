//! Rust parser of ZoneInfoDb(`tzdata`) on Android and OpenHarmony
//!
//! Ported from: https://android.googlesource.com/platform/prebuilts/fullsdk/sources/+/refs/heads/androidx-appcompat-release/android-34/com/android/i18n/timezone/ZoneInfoDb.java
use std::{
    ffi::CStr,
    fmt::Debug,
    fs::File,
    io::{Error, Read, Result, Seek, SeekFrom},
};

// The database reserves 40 bytes for each id.
const SIZEOF_TZNAME: usize = 40;
/// Ohos tzdata index entry size: `name + offset + length`
const SIZEOF_INDEX_ENTRY_OHOS: usize = SIZEOF_TZNAME + 2 * size_of::<u32>();
/// Android tzdata index entry size: `name + offset + length + raw_utc_offset(legacy)`:
/// [reference](https://android.googlesource.com/platform/prebuilts/fullsdk/sources/+/refs/heads/androidx-appcompat-release/android-34/com/android/i18n/timezone/ZoneInfoDb.java#271)
const SIZEOF_INDEX_ENTRY_ANDROID: usize = SIZEOF_TZNAME + 3 * size_of::<u32>();

/// Header of the `tzdata` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TzDataHeader {
    pub version: [u8; 5],
    pub index_offset: u32,
    pub data_offset: u32,
    pub zonetab_offset: u32,
}

impl TzDataHeader {
    /// Parse the header of the `tzdata` file.
    pub fn new<R: Read>(mut data: R) -> Result<Self> {
        /// e.g. `tzdata2024b\0`
        const TZDATA_VERSION_SIZE: usize = 12;
        /// Magic header of `tzdata` file
        const TZDATA_MAGIC_HEADER: &[u8] = b"tzdata";

        let version = {
            let mut magic = [0; TZDATA_VERSION_SIZE];
            data.read_exact(&mut magic)?;
            if !magic.starts_with(TZDATA_MAGIC_HEADER) || magic[TZDATA_VERSION_SIZE - 1] != 0 {
                return Err(Error::other("invalid tzdata header magic"));
            }
            let mut version = [0; 5];
            version.copy_from_slice(&magic[6..11]);
            version
        };

        let mut offset = [0; 4];
        data.read_exact(&mut offset)?;
        let index_offset = u32::from_be_bytes(offset);
        data.read_exact(&mut offset)?;
        let data_offset = u32::from_be_bytes(offset);
        data.read_exact(&mut offset)?;
        let zonetab_offset = u32::from_be_bytes(offset);

        Ok(Self { version, index_offset, data_offset, zonetab_offset })
    }
}

/// Index entry of the `tzdata` file.
pub struct TzDataIndex {
    pub name: Box<[u8]>,
    pub offset: u32,
    pub length: u32,
}

impl Debug for TzDataIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TzDataIndex")
            .field("name", &String::from_utf8_lossy(&self.name))
            .field("offset", &self.offset)
            .field("length", &self.length)
            .finish()
    }
}

/// Indexes of the `tzdata` file.
pub struct TzDataIndexes {
    indexes: Vec<TzDataIndex>,
}

impl TzDataIndexes {
    /// Parse the indexes of the `tzdata` file of Android.
    pub fn new_android<R: Read>(reader: R, header: &TzDataHeader) -> Result<Self> {
        Self::new::<SIZEOF_INDEX_ENTRY_ANDROID, R>(reader, header)
    }

    /// Parse the indexes of the `tzdata` file of HarmonyOS NEXT.
    pub fn new_ohos<R: Read>(reader: R, header: &TzDataHeader) -> Result<Self> {
        Self::new::<SIZEOF_INDEX_ENTRY_OHOS, R>(reader, header)
    }

    fn new<const SIZEOF_INDEX_ENTRY: usize, R: Read>(
        mut reader: R,
        header: &TzDataHeader,
    ) -> Result<Self> {
        let mut buf = vec![0; header.data_offset.saturating_sub(header.index_offset) as usize];
        reader.read_exact(&mut buf)?;
        // replace chunks with array_chunks when it's stable
        Ok(TzDataIndexes {
            indexes: buf
                .chunks(SIZEOF_INDEX_ENTRY)
                .filter_map(|chunk| {
                    if let Ok(name) = CStr::from_bytes_until_nul(&chunk[..SIZEOF_TZNAME]) {
                        let name = name.to_bytes().to_vec().into_boxed_slice();
                        let offset = u32::from_be_bytes(
                            chunk[SIZEOF_TZNAME..SIZEOF_TZNAME + 4].try_into().unwrap(),
                        );
                        let length = u32::from_be_bytes(
                            chunk[SIZEOF_TZNAME + 4..SIZEOF_TZNAME + 8].try_into().unwrap(),
                        );
                        Some(TzDataIndex { name, offset, length })
                    } else {
                        None
                    }
                })
                .collect(),
        })
    }

    /// Get all timezones.
    pub fn timezones(&self) -> &[TzDataIndex] {
        &self.indexes
    }

    /// Find a timezone by name.
    pub fn find_timezone(&self, timezone: &[u8]) -> Option<&TzDataIndex> {
        // timezones in tzdata are sorted by name.
        self.indexes.binary_search_by_key(&timezone, |x| &x.name).map(|x| &self.indexes[x]).ok()
    }

    /// Retrieve a chunk of timezone data by the index.
    pub fn find_tzdata<R: Read + Seek>(
        &self,
        mut reader: R,
        header: &TzDataHeader,
        index: &TzDataIndex,
    ) -> Result<Vec<u8>> {
        reader.seek(SeekFrom::Start(index.offset as u64 + header.data_offset as u64))?;
        let mut buffer = vec![0; index.length as usize];
        reader.read_exact(&mut buffer)?;
        Ok(buffer)
    }
}

/// Get timezone data from the `tzdata` file reader of Android.
pub fn find_tz_data_android(
    mut reader: impl Read + Seek,
    tz_name: &[u8],
) -> Result<Option<Vec<u8>>> {
    let header = TzDataHeader::new(&mut reader)?;
    let index = TzDataIndexes::new_android(&mut reader, &header)?;
    Ok(if let Some(entry) = index.find_timezone(tz_name) {
        Some(index.find_tzdata(reader, &header, entry)?)
    } else {
        None
    })
}

/// Get timezone data from the `tzdata` file reader of HarmonyOS NEXT.
pub fn find_tz_data_ohos(mut reader: impl Read + Seek, tz_name: &[u8]) -> Result<Option<Vec<u8>>> {
    let header = TzDataHeader::new(&mut reader)?;
    let index = TzDataIndexes::new_ohos(&mut reader, &header)?;
    Ok(if let Some(entry) = index.find_timezone(tz_name) {
        Some(index.find_tzdata(reader, &header, entry)?)
    } else {
        None
    })
}

/// Get timezone data from the `tzdata` file of Android.
pub fn find_tz_data_android_from_fs(tz_string: &str) -> Result<Option<Vec<u8>>> {
    fn open_android_tz_data_file() -> Result<File> {
        struct TzdataLocation {
            env_var: &'static str,
            path: &'static str,
        }

        const TZDATA_LOCATIONS: [TzdataLocation; 2] = [
            TzdataLocation { env_var: "ANDROID_DATA", path: "/misc/zoneinfo" },
            TzdataLocation { env_var: "ANDROID_ROOT", path: "/usr/share/zoneinfo" },
        ];

        for location in &TZDATA_LOCATIONS {
            if let Ok(env_value) = std::env::var(location.env_var) {
                if let Ok(file) = File::open(format!("{}{}/tzdata", env_value, location.path)) {
                    return Ok(file);
                }
            }
        }
        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    }
    let mut file = open_android_tz_data_file()?;
    find_tz_data_android(&mut file, tz_string.as_bytes())
}

/// Get timezone data from the `tzdata` file of HarmonyOS NEXT.
pub fn find_tz_data_ohos_from_fs(tz_string: &str) -> Result<Option<Vec<u8>>> {
    const TZDATA_PATH: &str = "/system/etc/zoneinfo/tzdata";
    match File::open(TZDATA_PATH) {
        Ok(mut file) => Ok(find_tz_data_ohos(&mut file, tz_string.as_bytes())?),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ohos_tzdata_header_and_index() {
        let file = File::open("./tests/ohos/tzdata").unwrap();
        let header = TzDataHeader::new(&file).unwrap();
        assert_eq!(header.version, *b"2024a");
        assert_eq!(header.index_offset, 24);
        assert_eq!(header.data_offset, 21240);
        assert_eq!(header.zonetab_offset, 272428);

        let iter = TzDataIndexes::new_ohos(&file, &header).unwrap();
        assert_eq!(iter.timezones().len(), 442);
        assert!(iter.find_timezone(b"Asia/Shanghai").is_some());
        assert!(iter.find_timezone(b"Pacific/Noumea").is_some());
    }

    #[test]
    fn test_ohos_tzdata_loading() {
        let file = File::open("./tests/ohos/tzdata").unwrap();
        let header = TzDataHeader::new(&file).unwrap();
        let iter = TzDataIndexes::new_ohos(&file, &header).unwrap();
        let timezone = iter.find_timezone(b"Asia/Shanghai").unwrap();
        let tzdata = iter.find_tzdata(&file, &header, timezone).unwrap();
        assert_eq!(tzdata.len(), 393);
    }

    #[test]
    fn test_android_tzdata_header_and_index() {
        let file = File::open("./tests/android/tzdata").unwrap();
        let header = TzDataHeader::new(&file).unwrap();
        assert_eq!(header.version, *b"2021a");
        assert_eq!(header.index_offset, 24);
        assert_eq!(header.data_offset, 30860);
        assert_eq!(header.zonetab_offset, 491837);

        let iter = TzDataIndexes::new_android(&file, &header).unwrap();
        assert_eq!(iter.timezones().len(), 593);
        assert!(iter.find_timezone(b"Asia/Shanghai").is_some());
        assert!(iter.find_timezone(b"Pacific/Noumea").is_some());
    }

    #[test]
    fn test_android_tzdata_loading() {
        let file = File::open("./tests/android/tzdata").unwrap();
        let header = TzDataHeader::new(&file).unwrap();
        let iter = TzDataIndexes::new_android(&file, &header).unwrap();
        let timezone = iter.find_timezone(b"Asia/Shanghai").unwrap();
        let tzdata = iter.find_tzdata(&file, &header, timezone).unwrap();
        assert_eq!(tzdata.len(), 573);
    }

    #[test]
    fn test_ohos_tzdata_find() {
        let file = File::open("./tests/ohos/tzdata").unwrap();
        let tzdata = find_tz_data_ohos(file, b"Asia/Shanghai").unwrap().unwrap();
        assert_eq!(tzdata.len(), 393);
    }

    #[test]
    fn test_android_tzdata_find() {
        let file = File::open("./tests/android/tzdata").unwrap();
        let tzdata = find_tz_data_android(file, b"Asia/Shanghai").unwrap().unwrap();
        assert_eq!(tzdata.len(), 573);
    }

    #[cfg(target_env = "ohos")]
    #[test]
    fn test_ohos_machine_tz_data_loading() {
        let file = File::open("/system/etc/zoneinfo/tzdata").unwrap();
        let tzdata = find_tz_data_ohos(file, b"Asia/Shanghai").unwrap().unwrap();
        assert!(!tzdata.is_empty());
    }
}
