use std::path::Path;
use std::fs;
use std::io::prelude::*;
use encoding_rs::{SHIFT_JIS, UTF_8};

const DVD_HEADER_SIZE: usize = 0x0440;
const DVD_MAGIC_NUMBER: u32 = 0xC2339F3D;
const DVD_IMAGE_SIZE: u64 = 1_459_978_240;
const GAME_NAME_SIZE: usize = 0x03e0;
const CONSOLE_ID: u8 = 0x47; //'G' in ASCII
const FILE_ENTRY_SIZE: usize = 0x0C;
const BANNER_NAME: &str = "opening.bnr";
const BANNER_SZ: usize = 6_496;

#[derive(Copy, Clone)]
pub enum Region {
    USA,
    EUR,
    JPN,
    FRA
}

impl Region {
    fn from_byte(byte: u8) -> Result<Region, &'static str> {
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
                Err("invalid region code")
            }
        }
    }
}

pub struct GCImage {
    pub header: DVDHeader,
    pub banner: Banner,
    pub region: Region
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

struct FileData {
    file_offset: u32,
    file_length: u32
}

struct DirData {
    parent_offset: u32,
    next_offset: u32
}

struct RootDirectory {
    num_entries: u32,
    string_table_ofst: u32
}

enum EntryType {
    File(FileData),
    Directory(DirData)
}

struct Entry {
    filename_ofst: u32,
    entry: EntryType
}

impl GCImage {
    pub fn open(path: &Path) -> Result<GCImage, &'static str> {
        let metadata = fs::metadata(path).unwrap();
        if metadata.len() != DVD_IMAGE_SIZE {
            return Err("invalid image size");
        }
        let mut file = fs::File::open(path).unwrap();
        file.seek(std::io::SeekFrom::Start(0)).unwrap();

        //Read and parse DVD Image header
        let mut data: [u8; DVD_HEADER_SIZE] = [0; DVD_HEADER_SIZE];
        file.read_exact(&mut data).unwrap();
        let header = parse_header(&data);
        validate_header(&header)?;

        let region = Region::from_byte(header.game_code[3])?;

        //Read and parse banner file. TODO, don't spam list files here. Maybe return an Iterator to each file entry?
        let root_entry = read_root_entry(&mut file, header.fst_ofst);
        list_files(&mut file, header.fst_ofst, &root_entry);
        let banner = read_banner(&mut file, header.fst_ofst, &root_entry, region)?;
        validate_banner(&banner)?;
        Ok(GCImage {
            header,
            banner,
            region
        })
    }
}

fn parse_header(data: &[u8]) -> DVDHeader {
    assert!(data.len() >= DVD_HEADER_SIZE);
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

fn read_banner(file: &mut fs::File, fst_ofst: u32, root_entry: &RootDirectory, region: Region) -> Result<Banner, &'static str> {
    let banner_entry = find_file(file, fst_ofst, root_entry, BANNER_NAME).unwrap();
    match banner_entry.entry {
        EntryType::File(file_data) => {
            let mut data = [0; BANNER_SZ];
            if file_data.file_length as usize != BANNER_SZ {
                return Err("malformed banner file")
            }
            file.seek(std::io::SeekFrom::Start(file_data.file_offset as u64)).unwrap();
            file.read_exact(&mut data).unwrap();

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
        _ => {
            Err("no opening.bnr found")
        }
    }
}

fn read_root_entry(file: &mut fs::File, fst_ofst: u32) -> RootDirectory {
    file.seek(std::io::SeekFrom::Start(fst_ofst as u64)).unwrap();
    let mut data = [0; FILE_ENTRY_SIZE];
    file.read_exact(&mut data).unwrap();

    let flags = data[0];
    assert!(flags == 1); //Root Entry Should always be a directory
    let num_entries = u8_arr_to_u32(&data[0x08..0x0C]);
    let string_table_ofst = num_entries * FILE_ENTRY_SIZE as u32;

    RootDirectory {
        num_entries,
        string_table_ofst
    }
}

fn read_entry(file: &mut fs::File, ofst: u32) -> Entry {
    file.seek(std::io::SeekFrom::Start(ofst as u64)).unwrap();
    let mut data = [0; FILE_ENTRY_SIZE];
    file.read_exact(&mut data).unwrap();

    let flags = data[0];
    let filename_ofst = u8_arr_to_u24(&data[0x01..0x04]);
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

    Entry {
        entry,
        filename_ofst
    }
}

fn list_files(file: &mut fs::File, fst_ofst: u32, root_entry: &RootDirectory) {
    for i in 0..root_entry.num_entries {
        let ofst = ( i * FILE_ENTRY_SIZE as u32 ) + fst_ofst;
        let entry = read_entry(file, ofst);
        let ofst = entry.filename_ofst + root_entry.string_table_ofst + fst_ofst;
        let filename = read_string(file, ofst as u64);
        let offsets = match entry.entry {
            EntryType::File(file_data) => {
                format!("File Offset: {}, File Length: {}", file_data.file_offset, file_data.file_length)
            },
            EntryType::Directory(dir_data) => {
                format!("Parent Offset: {}, Next Offset: {}", dir_data.parent_offset, dir_data.next_offset)
            }
        };
        println!("{:03} - {} - {}", i, filename, offsets);
    }
}

fn find_file(img_file: &mut fs::File, fst_ofst: u32, root_entry: &RootDirectory, name: &str) -> Option<Entry> {
    for i in 0..root_entry.num_entries {
        let ofst = ( i * FILE_ENTRY_SIZE as u32 ) + fst_ofst;
        let entry = read_entry(img_file, ofst);
        let ofst = entry.filename_ofst + root_entry.string_table_ofst + fst_ofst;
        let filename = read_string(img_file, ofst as u64);
        match entry.entry {
            EntryType::File(_) => {
                if filename == name {
                    return Some(entry);
                }
            }
            _ => {}
        }
    }
    None
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

fn validate_header(hdr: &DVDHeader) -> Result<(), &'static str> {
    if hdr.magic_word != DVD_MAGIC_NUMBER {
        return Err("incorrect or missing magic number");
    }
    if (hdr.fst_ofst as u64) >= DVD_IMAGE_SIZE {
        return Err("malformed filesystem table offset");
    }
    if (hdr.dol_ofst as u64) >= DVD_IMAGE_SIZE {
        return Err("malformed bootfile offset");
    }
    if hdr.game_code[0] != CONSOLE_ID {
        return Err("incorrect console id");
    }
    Ok(())
}

fn validate_banner(bnr: &Banner) -> Result<(), &'static str> {
    if bnr.magic_word[0] != b'B' ||
       bnr.magic_word[1] != b'N' ||
       bnr.magic_word[2] != b'R' ||
       ( bnr.magic_word[3] != b'1' && bnr.magic_word[3] != b'2' ) {
        Err("invalid banner magic word")
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
