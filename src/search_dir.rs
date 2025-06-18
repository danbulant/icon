use crate::icon::IconFile;
use crate::theme::{ThemeDescriptor, ThemeParseError};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// Icons and icon themes are looked for in a set of directories.
///
/// By default, that is `$HOME/.icons`, `$XDG_DATA_DIRS/icons` and `/usr/share/pixmaps`.
/// Applications may further add their own icon directories to this list, and users may extend or change the list.
/// The default list may be obtained using the `Default` implementation on `SearchDirectories` or its `default` method.
///
/// To add directories to the instance, use [SearchDirectories::append].
///
/// To construct a new `SearchDirectories` from a list, use the `From` implementation or construct it by hand.
///
/// # Example
///
/// ```
/// use icon::SearchDirectories;
///
/// let dirs = SearchDirectories::default();
/// // TODO
/// ```
#[derive(Debug, Clone)]
pub struct SearchDirectories {
    pub dirs: Vec<PathBuf>,
}

impl SearchDirectories {
    pub fn default() -> Self {
        <Self as Default>::default()
    }

    /// Add a list of directories to this `SearchDirectories`
    ///
    /// # Example
    ///
    /// ```
    /// use icon::SearchDirectories;
    ///
    /// let dirs = SearchDirectories::default().append(["/home/root/.icons"]);
    /// ```
    pub fn append<I, P>(mut self, directories: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        let mut extra_dirs = directories.into_iter().map(Into::into).collect();
        self.dirs.append(&mut extra_dirs);

        extra_dirs.into()
    }

    pub fn find_icon_locations(&self) -> IconLocations {
        // "Each theme is stored as subdirectories of the base directories"

        let (files, dirs) = self
            .dirs
            .iter()
            .flat_map(|base_dir| base_dir.read_dir()) // read the entries in each base dir
            .flatten() // merge all the iterators
            .flatten() // remove Err entries
            .filter_map(|entry| Some((entry.file_type().ok()?, entry))) // get file type for each entry and skip if fail
            .partition::<Vec<_>, _>(|(ft, _)| ft.is_file());

        // icons at the top-level in a base_dir don't belong to a theme, but must still be able to be found!
        let files = files
            .into_iter()
            .flat_map(|(_, entry)| IconFile::from_path(&entry.path()))
            .collect::<Vec<_>>();

        // "In at least one of the theme directories there must be a file called
        // index.theme that describes the theme. The first index.theme found while
        // searching the base directories in order is used"

        // For each theme name, create a list of directories where it may be found:
        let mut themes_directories: HashMap<OsString, Vec<PathBuf>> = HashMap::new();
        for (_, dir) in dirs {
            let theme_name = dir.file_name();

            themes_directories
                .entry(theme_name)
                .or_default()
                .push(dir.path());
        }

        IconLocations {
            standalone_icons: files,
            themes_directories,
        }
    }
}

#[derive(Debug)]
pub struct IconLocations {
    pub standalone_icons: Vec<IconFile>,
    pub themes_directories: HashMap<OsString, Vec<PathBuf>>,
}

impl IconLocations {
    pub fn theme<S>(&self, internal_name: S) -> std::io::Result<ThemeDescriptor<'_>>
    where
        S: AsRef<OsStr>,
    {
        let internal_name = internal_name.as_ref();

        let theme = self
            .themes_directories
            .get(internal_name)
            .ok_or_else(|| std::io::Error::other(ThemeParseError::NotAnIconTheme))?;

        ThemeDescriptor::new_from_folders(
            internal_name.to_string_lossy().into_owned(),
            theme.clone(),
        )
    }

    pub fn standalone_icon<S>(&self, icon_name: S) -> Option<&IconFile>
    where
        S: AsRef<OsStr>,
    {
        let name = icon_name.as_ref();
        
        self.standalone_icons.iter()
            .find(|icon| icon.path.file_stem() == Some(name))
    }
}

/// Anything that turns into an iterator of things that can become paths, can be turned into a `SearchDirectories`.
impl<I, P> From<I> for SearchDirectories
where
    I: IntoIterator<Item = P>,
    P: Into<PathBuf>,
{
    fn from(value: I) -> Self {
        let dirs = value.into_iter().map(Into::into).collect();

        SearchDirectories { dirs }
    }
}

impl Default for SearchDirectories {
    fn default() -> Self {
        // "By default, apps should look in $HOME/.icons (for backwards compatibility),
        // in $XDG_DATA_DIRS/icons
        // and in /usr/share/pixmaps (in that order)."

        let xdg = xdg::BaseDirectories::new();

        let mut directories = vec![];

        if let Some(home) = std::env::home_dir() {
            directories.push(home.join(".icons"));
        }

        xdg.data_dirs
            .into_iter()
            .map(|data_dir| data_dir.join("icons"))
            .for_each(|dir| directories.push(dir));

        directories.push("/usr/share/pixmaps".into());

        directories.into()
    }
}

#[cfg(test)]
mod test {
    use crate::search_dir::SearchDirectories;

    // these tests assume certain applications are installed on the system they are ran on.

    #[test]
    fn test_find_standard_theme_and_icon() {
        let dirs = SearchDirectories::default();

        let locations = dirs.find_icon_locations();
        
        let descriptor = locations.theme("Adwaita").unwrap();
        assert_eq!(descriptor.index.name, "Adwaita");

        let icon = locations.standalone_icon("htop").unwrap();
        assert_eq!(icon.path.file_name(), Some("htop.png".as_ref()))
    }
}
