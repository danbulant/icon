use std::path::{Path, PathBuf};
use crate::icon::IconFile;

#[derive(Debug, Clone)]
pub struct SearchDirectories {
    dirs: Vec<PathBuf>,
}

impl SearchDirectories {
    pub fn default() -> Self {
        <Self as Default>::default()
    }

    pub fn search_icons_and_theme_indexes(&self) -> (Vec<IconFile>, Vec<PathBuf>) {
        fn theme_name_from_path(path: &Path) -> Option<&str> {
            let theme_name = path.components()
                .nth_back(1); // get the second-to-last component (which should be the theme name)

            Some(theme_name?.as_os_str().to_str()?)
        }

        // "Each theme is stored as subdirectories of the base directories"

        let (dirs, files) = self.dirs.iter()
            .flat_map(|base_dir| base_dir.read_dir()) // read the entries in each base dir
            .flat_map(std::convert::identity) // merge all the iterators
            .flat_map(std::convert::identity) // remove Err entries
            .filter_map(|entry| Some((entry.file_type().ok()?, entry))) // get file type for each entry and skip if fail
            .partition::<Vec<_>, _>(|(ft, _)| ft.is_dir());

        // icons at the top-level in a base_dir don't belong to a theme, but must still be able to be found!
        let files = files.into_iter()
            .flat_map(|(_, entry)| IconFile::from_path(&entry.path()))
            .collect::<Vec<_>>();

        // "In at least one of the theme directories there must be a file called
        // index.theme that describes the theme. The first index.theme found while
        // searching the base directories in order is used"

        let mut indexes = dirs.into_iter()
            .map(|(_, entry)| entry.path().join("index.theme"))
            .filter(|path| path.exists()) // the index.theme file must exist
            .collect::<Vec<_>>();

        // only keep the first `index.theme` for each theme
        indexes.dedup_by_key(|path| theme_name_from_path(&path).map(|s| s.to_string()));

        (files, indexes)
    }
}

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
    fn test_find_htop_icon() {
        let dirs = SearchDirectories::default();

        let (icons, _indexes) = dirs.search_icons_and_theme_indexes();

        assert!(icons.iter().any(|i| i.path.file_name().and_then(|s| s.to_str()) == Some("htop.png")))
    }
}
