use std::cell::RefCell;
use std::cmp::min;
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::io::{Read, Seek, SeekFrom};
use gloo_net::http::Request;
use crate::ebml::{EbmlSource, Size};

thread_local! {
    pub static FILE_CACHE: RefCell<HashMap<String, File>> = RefCell::new(
      HashMap::new()
    );
}

#[derive(Clone, Debug)]
pub struct FsSource {
    pub file_path: String
}


impl PartialEq for FsSource {
    fn eq(&self, other: &Self) -> bool {
        self.file_path == other.file_path
    }
}

impl FsSource {
    pub fn new(file_path: &str) -> FsSource {
        FsSource {
            file_path: file_path.to_string(),
        }
    }
}

fn read_byte_range(file_path: &str, start: u64, length: usize) -> io::Result<Vec<u8>> {
    //println!("{} - {}", start, length);
    FILE_CACHE.with(|cache| {
        let mut map = cache.borrow_mut();

        if !map.contains_key(file_path) {
            let file = File::open(file_path).expect("Failed to open file");
            map.insert(file_path.to_string(), file);
        }

        let file = map.get_mut(file_path).expect("We should have a file at this point!");

        file.seek(SeekFrom::Start(start))?;
        let mut buffer = vec![0; length];
        file.read_exact(&mut buffer)?;

        Ok(buffer)
    })
}

impl EbmlSource for FsSource {

    async fn read_range(&self, start: Size, end: Size) -> Vec<u8> {
        let data = read_byte_range(self.file_path.as_str(), start, (end - start + 1) as usize)
            .unwrap_or_else(|_| Vec::<u8>::new());

        data
    }

    async fn read_exact(&self, start: Size, length: usize) -> Vec<u8> {
        let data = read_byte_range(self.file_path.as_str(), start, length as usize)
            .unwrap_or_else(|_| Vec::<u8>::new());

        data
    }

}