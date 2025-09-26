use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IconFile {
    pub path: PathBuf,
    pub name: OsString,
    pub file_type: FileType,
}

impl IconFile {
    pub fn from_path(path: &Path) -> Option<IconFile> {
        let file_type = FileType::from_path_ext(path)?;
        let name = path.file_stem()?.to_os_string();

        Some(IconFile {
            path: path.to_owned(),
            file_type,
            name,
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FileType {
    Png,
    Xmp,
    Svg,
}

impl FileType {
    pub fn from_path_ext(path: &Path) -> Option<Self> {
        let ext = path.extension()?;
        let ext = ext.to_str()?;

        if ext.eq_ignore_ascii_case("png") {
            Some(FileType::Png)
        } else if ext.eq_ignore_ascii_case("xmp") {
            Some(FileType::Xmp)
        } else if ext.eq_ignore_ascii_case("svg") {
            Some(FileType::Svg)
        } else {
            None
        }
    }

    pub fn ext(&self) -> &str {
        match self {
            FileType::Png => "png",
            FileType::Xmp => "xmp",
            FileType::Svg => "svg",
        }
    }

    pub const fn types() -> [FileType; 3] {
        [FileType::Png, FileType::Xmp, FileType::Svg]
    }
}
