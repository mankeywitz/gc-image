use std::path::Path;
use std::fs;
use std::io::prelude::*;
use encoding_rs::{SHIFT_JIS, UTF_8};
use thiserror::Error;

const DVD_HEADER_SIZE: usize = 0x0440;
const DVD_MAGIC_NUMBER: u32 = 0xC2339F3D;
const DVD_IMAGE_SIZE: u64 = 1_459_978_240;
const GAME_NAME_SIZE: usize = 0x03e0;
const CONSOLE_ID: u8 = 0x47; //'G' in ASCII
const FILE_ENTRY_SIZE: usize = 0x0C;
const BANNER_NAME: &str = "opening.bnr";
const BANNER_SZ: usize = 6_496;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ImageError {
    #[error("error reading file")]
    IOError(#[from] std::io::Error),
    #[error("invalid image file")]
    InvalidFileType,
    #[error("invalid region byte: {byte}")]
    InvalidRegion {
        byte: u8
    },
    #[error("invalid image header ({0})")]
    InvalidHeader(String),
    #[error("invalid banner data({0})")]
    InvalidBanner(String),
    #[error("{0} was not found in the image")]
    FileNotFound(String),
}

#[derive(Copy, Clone)]
pub enum Region {
    USA,
    EUR,
    JPN,
    FRA
}

impl Region {
    fn from_byte(byte: u8) -> Result<Region, ImageError> {
        match byte {
            b'E' => {
                Ok(Region::USA)
            },
            b'J' => {
                Ok(Region::JPN)
            },
            b'P' => {
                Ok(Region::EUR)
            },
            b'F' => {
                Ok(Region::FRA)
            },
            _ => {
                Err(ImageError::InvalidRegion {
                    byte
                })
            }
        }
    }
}

pub struct GCImage {
    pub header: DVDHeader,
    pub banner: Banner,
    pub region: Region,
    file: fs::File
}

pub struct DVDHeader {
    pub game_code: [u8; 4],
    pub maker_code: [u8; 2],
    pub disk_id: u8,
    pub version: u8,
    pub audio_streaming: bool,
    pub stream_buf_sz: u8,
    pub magic_word: u32,
    pub game_name: String,
    pub dol_ofst: u32,
    pub fst_ofst: u32,
    pub fst_sz: u32,
    pub max_fst_sz: u32
}

pub struct Banner {
    pub magic_word: [u8; 4],
    pub graphical_data: [u8; 0x1800], //RGB5A1 format
    pub game_name: String,
    pub developer: String,
    pub full_game_title: String,
    pub full_developer_name: String,
    pub description: String
}

pub struct FileData {
    file_offset: u32,
    file_length: u32
}

pub struct DirData {
    parent_offset: u32,
    next_offset: u32
}

struct RootDirectory {
    num_entries: u32,
    string_table_ofst: u32
}

pub enum EntryType {
    File(FileData),
    Directory(DirData)
}

pub struct FilesystemEntry {
    pub filename: String,
    pub entry: EntryType
}

pub struct FilesystemTree {
    files: Vec<FilesystemEntry>
}

impl GCImage {
    pub fn open(path: &Path) -> Result<GCImage, ImageError> {
        let metadata = fs::metadata(path)?;
        if metadata.len() != DVD_IMAGE_SIZE {
            return Err(ImageError::InvalidFileType);
        }
        let mut file = fs::File::open(path)?;
        file.seek(std::io::SeekFrom::Start(0))?;

        //Read and parse DVD Image header
        let mut data: [u8; DVD_HEADER_SIZE] = [0; DVD_HEADER_SIZE];
        file.read_exact(&mut data)?;
        let header = parse_header(&data);
        validate_header(&header)?;

        let region = Region::from_byte(header.game_code[3])?;

        let root_entry = read_root_entry(&mut file, header.fst_ofst)?;
        let banner = read_banner(&mut file, header.fst_ofst, &root_entry, region)?;
        validate_banner(&banner)?;
        Ok(GCImage {
            header,
            banner,
            region,
            file
        })
    }

    pub fn files(&mut self) -> Result<FilesystemTree, ImageError> {
        let root_entry = read_root_entry(&mut self.file, self.header.fst_ofst)?;
        let str_tbl_ofst = self.header.fst_ofst + root_entry.string_table_ofst;
        let mut files = Vec::new();
        for i in 0..root_entry.num_entries {
            let ofst = (i * FILE_ENTRY_SIZE as u32) + self.header.fst_ofst;
            let entry = read_entry(&mut self.file, ofst, str_tbl_ofst)?;
            files.push(entry);
        }
        Ok(FilesystemTree {
            files
        })
    }
}

impl IntoIterator for FilesystemTree {
    type Item = FilesystemEntry;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.files.into_iter()
    }
}

fn parse_header(data: &[u8]) -> DVDHeader {
    assert!(data.len() == DVD_HEADER_SIZE);
    let mut game_code = [0; 4];
    game_code.clone_from_slice(&data[0..=0x3]);
    let mut maker_code = [0; 2]; 
    maker_code.clone_from_slice(&data[0x4..=0x5]);
    let disk_id = data[0x6];
    let version = data[0x7];
    let audio_streaming = data[0x8] != 0;
    let stream_buf_sz = data[0x9];
    let magic_word = u8_arr_to_u32( &data[0x001c..=0x001f] );
    let mut game_name = [0; GAME_NAME_SIZE];
    game_name.clone_from_slice(&data[0x0020..=0x03ff]);
    let game_name = String::from_utf8(game_name.to_vec()).unwrap();
    let dol_ofst = u8_arr_to_u32(&data[0x0420..=0x0423]);
    let fst_ofst = u8_arr_to_u32(&data[0x0424..=0x0427]);
    let fst_sz = u8_arr_to_u32(&data[0x0428..=0x042B]);
    let max_fst_sz = u8_arr_to_u32(&data[0x042C..=0x042F]);
    DVDHeader {
        game_code,
        maker_code,
        disk_id,
        version,
        audio_streaming,
        stream_buf_sz,
        magic_word,
        game_name,
        dol_ofst,
        fst_ofst,
        fst_sz,
        max_fst_sz
    }
}

fn read_banner(file: &mut fs::File, fst_ofst: u32, root_entry: &RootDirectory, region: Region) -> Result<Banner, ImageError> {
    let banner_entry = find_file(file, fst_ofst, root_entry, BANNER_NAME)?;
    match banner_entry.entry {
        EntryType::File(file_data) => {
            let mut data = [0; BANNER_SZ];
            if file_data.file_length as usize != BANNER_SZ {
                return Err(ImageError::InvalidBanner("malformed banner file".to_string()));
            }
            file.seek(std::io::SeekFrom::Start(file_data.file_offset as u64))?;
            file.read_exact(&mut data)?;

            let mut magic_word = [0; 0x4];
            magic_word.copy_from_slice(&data[0..0x4]);
            let mut graphical_data = [0; 0x1800];
            graphical_data.copy_from_slice(&data[0x0020..0x1820]);
            let game_name = byte_slice_to_string(&data[0x1820..0x1840], region);
            let developer = byte_slice_to_string(&data[0x1840..0x1860], region);
            let full_game_title = byte_slice_to_string(&data[0x1860..0x18a0], region);
            let full_developer_name = byte_slice_to_string(&data[0x18a0..0x18e0], region) ;
            let description = byte_slice_to_string(&data[0x18e0..0x1960], region);
            Ok(Banner {
                magic_word,
                graphical_data,
                game_name,
                developer,
                full_game_title,
                full_developer_name,
                description
            })
        },
        _ => { Err(ImageError::InvalidBanner("opening.bnr must be a file".to_string())) }
    }
}

fn read_root_entry(file: &mut fs::File, fst_ofst: u32) -> Result<RootDirectory, ImageError> {
    file.seek(std::io::SeekFrom::Start(fst_ofst as u64))?;
    let mut data = [0; FILE_ENTRY_SIZE];
    file.read_exact(&mut data)?;

    let flags = data[0];
    //Root Entry Should always be a directory
    if flags != 1 {
        return Err(ImageError::InvalidHeader("invalid root directory entry".to_string()));
    }
    let num_entries = u8_arr_to_u32(&data[0x08..0x0C]);
    let string_table_ofst = num_entries * FILE_ENTRY_SIZE as u32;

    Ok(RootDirectory {
        num_entries,
        string_table_ofst
    })
}

fn read_entry(file: &mut fs::File, ofst: u32, string_table_ofst: u32) -> Result<FilesystemEntry, ImageError> {
    file.seek(std::io::SeekFrom::Start(ofst as u64))?;
    let mut data = [0; FILE_ENTRY_SIZE];
    file.read_exact(&mut data)?;

    let flags = data[0];
    let filename_ofst = u8_arr_to_u24(&data[0x01..0x04]);
    let ofst = filename_ofst + string_table_ofst;
    let filename = read_string(file, ofst as u64);
    let entry = if flags == 0 {
        //File
        let file_offset = u8_arr_to_u32(&data[0x04..0x08]);
        let file_length = u8_arr_to_u32(&data[0x08..0x0C]);
        EntryType::File(FileData {
            file_offset,
            file_length
        })
    } else {
        //Directory
        let parent_offset = u8_arr_to_u32(&data[0x04..0x08]);
        let next_offset = u8_arr_to_u32(&data[0x08..0x0C]);
        EntryType::Directory(DirData {
            parent_offset,
            next_offset
        })
    };

    Ok(FilesystemEntry {
        entry,
        filename
    })
}

fn find_file(img_file: &mut fs::File, fst_ofst: u32, root_entry: &RootDirectory, name: &str) -> Result<FilesystemEntry, ImageError> {
    for i in 0..root_entry.num_entries {
        let ofst = ( i * FILE_ENTRY_SIZE as u32 ) + fst_ofst;
        let entry = read_entry(img_file, ofst, root_entry.string_table_ofst + fst_ofst)?;
        match entry.entry {
            EntryType::File(_) => {
                if entry.filename == name {
                    return Ok(entry);
                }
            }
            _ => {}
        }
    }
    Err(ImageError::FileNotFound(name.to_string()))
}

fn read_string(file: &mut fs::File, ofst: u64) -> String {
    let mut bytes = Vec::new();

    file.seek(std::io::SeekFrom::Start(ofst as u64)).unwrap();

    for byte in file.bytes() {
        let byte = byte.unwrap();
        if byte == 0 {
            break;
        }
        bytes.push(byte);
    }

    String::from_utf8(bytes).unwrap()
}

fn byte_slice_to_string(bytes: &[u8], region: Region) -> String {
    match region {
        Region::USA |
        Region::EUR |
        Region::FRA => {
            let (s, _, _) = UTF_8.decode(bytes);
            s.to_string()
        },
        Region::JPN => {
            let(s, _, _) = SHIFT_JIS.decode(&bytes);
            s.to_string()
        }
    }
}

fn validate_header(hdr: &DVDHeader) -> Result<(), ImageError> {
    if hdr.magic_word != DVD_MAGIC_NUMBER {
        return Err(ImageError::InvalidHeader("incorrect or missing magic number".to_string()));
    }
    if (hdr.fst_ofst as u64) >= DVD_IMAGE_SIZE {
        return Err(ImageError::InvalidHeader("malformed filesystem table offset".to_string()));
    }
    if (hdr.dol_ofst as u64) >= DVD_IMAGE_SIZE {
        return Err(ImageError::InvalidHeader("malformed bootfile offset".to_string()));
    }
    if hdr.game_code[0] != CONSOLE_ID {
        return Err(ImageError::InvalidHeader("incorrect console id".to_string()));
    }
    Ok(())
}

fn validate_banner(bnr: &Banner) -> Result<(), ImageError> {
    if bnr.magic_word[0] != b'B' ||
       bnr.magic_word[1] != b'N' ||
       bnr.magic_word[2] != b'R' ||
       ( bnr.magic_word[3] != b'1' && bnr.magic_word[3] != b'2' ) {
        Err(ImageError::InvalidBanner("invalid banner magic word".to_string()))
    } else {
        Ok(())
    }
}

fn u8_arr_to_u32(arr: &[u8]) -> u32 {
    assert!(arr.len() == 4);
    let b1 = ( arr[0]  as u32) << 24;
    let b2 = ( arr[1]  as u32) << 16;
    let b3 = ( arr[2]  as u32) << 8;
    let b4 = arr[3] as u32;
    b1 | b2 | b3 | b4
}

fn u8_arr_to_u24(arr: &[u8]) -> u32 {
    assert!(arr.len() == 3);
    let b1 = ( arr[0] as u32) << 16;
    let b2 = ( arr[1]  as u32 ) << 8;
    let b3 = arr[2] as u32;
    b1 | b2 | b3
}

#[cfg(test)]
mod tests {
    #[test]
    fn load_iso() {
        assert!(true);
    }
}
