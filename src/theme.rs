use crate::IconSearch;
use crate::icon::IconFile;
use crate::theme::ThemeParseError::MissingRequiredAttribute;
use freedesktop_entry_parser::low_level::{EntryIter, SectionBytes};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Icons {
    pub standalone_icons: Vec<IconFile>,
    pub themes: HashMap<OsString, Arc<Theme>>,
}

impl Icons {
    /// Creates a new `Icons`, performing a search in the standard directories.
    ///
    /// This function collects all standalone icons and icon themes on the system.
    /// To configure what directories are searched, use [`IconSearch`] instead.
    pub fn new() -> Self {
        IconSearch::default().search().icons()
    }

    pub fn theme(&self, theme_name: &str) -> Option<Arc<Theme>> {
        let theme_name: &OsStr = theme_name.as_ref();
        self.themes.get(theme_name).cloned()
    }

    pub fn find_default_icon(&self, icon_name: &str, size: u32, scale: u32) -> Option<IconFile> {
        self.find_icon(icon_name, size, scale, "hicolor")
    }

    /// Look up an icon by name, size, scale and theme.
    ///
    /// If the icon is not found in the theme, its parents are checked.
    /// If no theme by the given name exists, the `"hicolor"` theme (default theme) is checked.
    pub fn find_icon(
        &self,
        icon_name: &str,
        size: u32,
        scale: u32,
        theme: &str,
    ) -> Option<IconFile> {
        let theme = self.theme(theme).or_else(|| self.theme("hicolor"))?;
        theme
            .find_icon(icon_name, size, scale)
            .or_else(|| self.find_standalone_icon(icon_name))
    }

    pub fn find_standalone_icon(&self, icon_name: &str) -> Option<IconFile> {
        self.standalone_icons
            .iter()
            .find(|ico| ico.path.file_stem() == Some(icon_name.as_ref()))
            .cloned()
    }
}

pub struct Theme {
    pub info: ThemeInfo,
    pub inherits_from: Vec<Arc<Theme>>,
}

impl Theme {
    pub fn find_icon_unscaled(&self, icon_name: &str, size: u32) -> Option<IconFile> {
        self.find_icon(icon_name, size, 1)
    }

    pub fn find_icon(&self, icon_name: &str, size: u32, scale: u32) -> Option<IconFile> {
        self.find_icon_here(icon_name, size, scale).or_else(|| {
            // or find it in one of our parents
            self.inherits_from
                .iter()
                .find_map(|theme| theme.find_icon_here(icon_name, size, scale))
        })
    }

    fn find_icon_here(&self, icon_name: &str, size: u32, scale: u32) -> Option<IconFile> {
        const EXTENSIONS: [&'static str; 3] = ["png", "xmp", "svg"];
        let file_names = EXTENSIONS.map(|ext| format!("{icon_name}.{ext}"));

        let base_dirs = &self.info.base_dirs;

        let sub_dirs = &self.info.index.directories;
        // first, try to find an exact icon size match:
        let exact_sub_dirs = sub_dirs
            .into_iter()
            .filter(|sub_dir| sub_dir.matches_size(size, scale));

        for base_dir in base_dirs {
            for sub_dir in exact_sub_dirs.clone() {
                for file_name in &file_names {
                    let path = base_dir
                        .join(sub_dir.directory_name.as_str())
                        .join(file_name);

                    if path.exists() {
                        if let Some(file) = IconFile::from_path(&path) {
                            // exact match!
                            return Some(file);
                        }
                    }
                }
            }
        }

        drop(exact_sub_dirs);

        // no exact match: try to find a match as close as possible instead.
        let mut min_dist = u32::MAX;
        let mut best_icon = None;

        for base_dir in base_dirs {
            for sub_dir in sub_dirs {
                let distance = sub_dir.size_distance(size, scale);

                if distance < min_dist {
                    for file_name in &file_names {
                        let path = base_dir
                            .join(sub_dir.directory_name.as_str())
                            .join(file_name);
                        if path.exists() {
                            if let Some(file) = IconFile::from_path(&path) {
                                min_dist = distance;
                                best_icon = Some(file);
                            }
                        }
                    }
                }
            }
        }

        best_icon
    }
}

pub struct ThemeInfo {
    pub internal_name: String,
    pub base_dirs: Vec<PathBuf>,
    pub index_location: PathBuf,
    pub index: ThemeIndex,
    // additional groups?
}

#[derive(Debug, thiserror::Error)]
pub enum ThemeParseError {
    #[error("missing Icon Theme index or section")]
    NotAnIconTheme,
    #[error("missing attribute `{0}`")]
    MissingRequiredAttribute(&'static str),
    #[error("the input wasn't in utf-8")]
    NotUtf8(#[from] std::str::Utf8Error),
    #[error("a bool was expected but failed to parse")]
    ParseBoolError(#[from] std::str::ParseBoolError),
    #[error("a number was expected but failed to parse")]
    ParseNumError(#[from] std::num::ParseIntError),
    #[error("A directory type was invalid")]
    InvalidDirectoryType,
    #[error("invalid format for a freedesktop entry file")]
    ParseError(#[from] freedesktop_entry_parser::ParseError),
}

impl ThemeInfo {
    pub fn new_from_folders(internal_name: String, folders: Vec<PathBuf>) -> std::io::Result<Self> {
        let index_location = folders
            .iter()
            .map(|f| f.join("index.theme"))
            .find(|index_path| index_path.exists())
            .ok_or_else(|| std::io::Error::other(ThemeParseError::NotAnIconTheme))?;

        let index = ThemeIndex::parse_from_file(index_location.as_path())?;

        Ok(Self {
            internal_name,
            base_dirs: folders,
            index_location,
            index,
        })
    }
}

pub struct ThemeIndex {
    pub name: String,
    pub comment: String,
    pub inherits: Vec<String>,
    pub directories: Vec<DirectoryIndex>,
    pub hidden: bool,
    pub example: Option<String>,
}

impl ThemeIndex {
    pub fn parse_from_file(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let index = ThemeIndex::parse(&bytes).map_err(std::io::Error::other)?;

        Ok(index)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, ThemeParseError> {
        let mut entry: EntryIter = freedesktop_entry_parser::low_level::parse_entry(bytes);

        let icon_theme_section: SectionBytes =
            entry.next().ok_or(ThemeParseError::NotAnIconTheme)??;
        let name: &str = find_attr_req(&icon_theme_section, "Name")?;

        // SPEC: `Comment` is required, but most icon theme developers can't be arsed to
        // include it! To make `icon` practical, we choose a default of an empty string instead.
        // `let comment = find_attr_req(&icon_theme_section, "Comment")?;`
        let comment = find_attr(&icon_theme_section, "Comment")?.unwrap_or("");
        // If no theme is specified, implementations are required to add the "hicolor" theme to the inheritance tree.
        let inherits = find_attr(&icon_theme_section, "Inherits")?
            .iter()
            .flat_map(|s| s.split(',')) // `inherits` is a comma-separated string list
            .map(Into::into)
            .collect::<Vec<_>>();
        let directories = find_attr_req(&icon_theme_section, "Directories")?
            .split(',')
            .collect::<Vec<_>>();
        let scaled_directories = find_attr(&icon_theme_section, "ScaledDirectories")?
            .map(|s| s.split(',').collect::<Vec<_>>());
        let hidden = find_attr(&icon_theme_section, "Hidden")?
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(false);
        let example = find_attr(&icon_theme_section, "Example")?;

        // all other sections should describe a directory in the directory list
        let directories = entry
            .filter_map(Result::ok)
            .filter_map(|section| {
                let title = str::from_utf8(section.title).ok()?;

                let is_scaled_dir = scaled_directories
                    .as_ref()
                    .map(|d| d.contains(&title))
                    .unwrap_or(false);

                if !directories.contains(&title) && !is_scaled_dir {
                    // this section isn't a listed directory! ignore!
                    return None;
                }

                let mut index = DirectoryIndex::parse(section);

                if is_scaled_dir {
                    if let Ok(index) = &mut index {
                        index.is_scaled_dir = true;
                    }
                }

                Some(index)
            })
            .collect::<Result<Vec<_>, ThemeParseError>>()?;

        Ok(Self {
            name: name.into(),
            comment: comment.into(),
            inherits,
            directories,
            hidden,
            example: example.map(Into::into),
        })
    }
}

pub struct DirectoryIndex {
    pub directory_name: String,
    pub is_scaled_dir: bool,
    pub size: u32,
    pub scale: u32,
    pub context: Option<String>,
    pub directory_type: DirectoryType,
    pub max_size: u32,
    pub min_size: u32,
    pub threshold: u32,
    // pub additional_values: HashMap<String, String>,
}

impl DirectoryIndex {
    fn parse(section: SectionBytes) -> Result<Self, ThemeParseError> {
        let dir_name = str::from_utf8(section.title)?;
        let size: u32 = find_attr_req(&section, "Size")?.parse()?;
        let scale: u32 = find_attr(&section, "Scale")?
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(1);
        let context = find_attr(&section, "Context")?;
        // Valid types are Fixed, Scalable and Threshold.
        // The type decides what other keys in the section are used.
        // If not specified, the default is Threshold.
        let directory_type = find_attr(&section, "Type")?
            .map(|s| s.try_into())
            .transpose()
            .map_err(|_| ThemeParseError::InvalidDirectoryType)?
            .unwrap_or(DirectoryType::Threshold);
        let max_size = find_attr(&section, "MaxSize")?
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(size);
        let min_size = find_attr(&section, "MinSize")?
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(size);
        let threshold = find_attr(&section, "Threshold")?
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(2);

        Ok(Self {
            directory_name: dir_name.into(),
            is_scaled_dir: scale != 1,
            size,
            scale,
            context: context.map(Into::into),
            directory_type,
            max_size,
            min_size,
            threshold,
        })
    }

    fn size_distance(&self, icon_size: u32, icon_scale: u32) -> u32 {
        let size = icon_size * icon_scale;

        match self.directory_type {
            DirectoryType::Fixed | DirectoryType::Scalable => {
                (self.size * self.scale).abs_diff(size)
            }
            DirectoryType::Threshold => {
                let lower = (self.size - self.threshold) * self.scale;
                let higher = (self.size + self.threshold) * self.scale;

                if size < lower {
                    size.abs_diff(self.min_size * self.scale)
                } else if size > higher {
                    size.abs_diff(self.max_size * self.scale)
                } else {
                    0 // within range -> no distance!
                }
            }
        }
    }

    pub fn matches_size(&self, icon_size: u32, icon_scale: u32) -> bool {
        if self.scale != icon_scale {
            return false;
        }

        match self.directory_type {
            DirectoryType::Fixed => self.size == icon_size,
            DirectoryType::Scalable => {
                let DirectoryIndex {
                    min_size, max_size, ..
                } = *self;

                (min_size..=max_size).contains(&icon_size)
            }
            DirectoryType::Threshold => {
                let DirectoryIndex {
                    threshold, size, ..
                } = *self;

                // The icons in this directory can be used if the size differ at most this much from the desired (unscaled) size
                size.abs_diff(icon_size) <= threshold
            }
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum DirectoryType {
    Fixed,
    Scalable,
    Threshold,
}

impl TryFrom<&str> for DirectoryType {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let value = match value {
            "Fixed" => DirectoryType::Fixed,
            "Scalable" => DirectoryType::Scalable,
            "Threshold" => DirectoryType::Threshold,
            _ => return Err(()),
        };

        Ok(value)
    }
}

fn find_attr<'a>(
    section: &'a SectionBytes,
    name: &str,
) -> Result<Option<&'a str>, std::str::Utf8Error> {
    section
        .attrs
        .iter()
        .find(|attr| attr.name == name.as_bytes() && attr.param.is_none())
        .map(|attr| str::from_utf8(attr.value))
        .transpose()
}

fn find_attr_req<'a>(
    section: &'a SectionBytes,
    name: &'static str,
) -> Result<&'a str, ThemeParseError> {
    find_attr(section, name)?.ok_or(MissingRequiredAttribute(name))
}

#[cfg(test)]
mod test {
    use crate::Icons;
    use crate::icon::{FileType, IconFile};
    use crate::theme::{DirectoryType, ThemeIndex};
    use std::error::Error;
    use std::path::Path;

    #[test]
    fn test_find_firefox() {
        let icons = Icons::new();

        let ico = icons.find_default_icon("firefox", 128, 1);

        assert_eq!(
            ico,
            Some(IconFile {
                path: "/usr/share/icons/hicolor/128x128/apps/firefox.png".into(),
                file_type: FileType::Png
            })
        );

        // we should be able to find an icon for a bunch of different sizes
        for size in (16u32..=64).step_by(8) {
            assert!(icons.find_default_icon("firefox", size, 1).is_some());
        }

        assert!(icons.find_default_icon("firefox", 64, 2).is_some());
    }

    #[test]
    fn find_all_desktop_entry_icons() {
        let icons = Icons::new();

        // some desktop files are just packaged poorly.
        // if a test fails here, and you are certain that the icon just straight up doesn't exist,
        // or is in an unfindable place by normal means,
        // disallow it in this list.
        static DISALLOW_LIST: &[&str] = &[
            "imv-dir",
            "imv",
            "io.elementary.granite.demo",
            "java-java-openjdk",
            "jconsole-java-openjdk",
            "jshell-java-openjdk",
            "lstopo",
            "signon-ui",
        ];

        for entry in
            freedesktop_desktop_entry::Iter::new(freedesktop_desktop_entry::default_paths())
                .entries(None::<&[&str]>)
        {
            let Some(icon_name) = entry.icon() else {
                continue;
            };

            if Path::new(icon_name).exists() {
                continue; // absolute URLs to icons are OK
            }

            if DISALLOW_LIST
                .iter()
                .any(|x| Some(x.as_ref()) == entry.path.file_stem())
            {
                continue;
            }

            // TODO: perhaps our system should expose a way to construct a "composed theme" filter,
            // for cases where you want to search a multitude (or all) themes
            let icon = icons
                .find_icon(icon_name, 32, 1, "gnome")
                .or_else(|| icons.find_icon(icon_name, 32, 1, "breeze"));

            assert!(
                icon.is_some(),
                "Icon {icon_name} from desktop entry {:?} missing!!",
                entry.path
            )
        }
    }

    #[test]
    fn test_parse_example_theme() -> Result<(), Box<dyn Error>> {
        static EXAMPLE: &'static str = include_str!("../resources/example.index.theme");

        let index = ThemeIndex::parse(EXAMPLE.as_bytes())?;

        assert_eq!(index.name, "Birch");
        assert_eq!(index.comment, "Icon theme with a wooden look");
        assert_eq!(index.inherits, vec!["wood", "default"]);

        let directories = index.directories;

        assert_eq!(directories.len(), 7);

        let first_dir_index = &directories[0];
        assert_eq!(first_dir_index.directory_name, "scalable/apps");
        assert_eq!(first_dir_index.is_scaled_dir, false);
        assert_eq!(first_dir_index.size, 48);
        assert_eq!(first_dir_index.scale, 1);
        assert_eq!(first_dir_index.context.as_deref(), Some("Applications"));
        assert_eq!(first_dir_index.directory_type, DirectoryType::Scalable);
        assert_eq!(first_dir_index.max_size, 256);
        assert_eq!(first_dir_index.min_size, 1);
        assert_eq!(first_dir_index.threshold, 2);

        assert_eq!(index.hidden, false);
        assert_eq!(index.example, None);

        Ok(())
    }
}
