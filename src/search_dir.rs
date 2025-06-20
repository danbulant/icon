use crate::icon::IconFile;
use crate::theme::{OwnedThemeDescriptor, Theme, ThemeDescriptor, ThemeParseError};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Arc;

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
    pub fn resolve(self) -> Vec<Arc<Theme<'static>>> {
        let names = self.themes_directories.keys().cloned().collect::<Vec<_>>();

        self.resolve_only(names)
    }

    pub fn resolve_only<I, S>(self, theme_names: I) -> Vec<Arc<Theme<'static>>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        // Icon themes may transitively depend on the same icon theme many times.
        // This is a bit of an issue, as when an exhaustive icon lookup would be implemented naively,
        // users may end up searching the same icon theme multiple times.
        // To accommodate this, either one has to keep a list of visited icon themes every time they
        // perform a lookup, or avoid the issue altogether by removing redundant parents up-front.

        // That second option is what this function does, paying a (rather small) one-time cost to
        // make the rest of the API cleaner and smaller by guaranteeing that the returned icon themes
        // have dependencies that form a direct acyclic graph without redundant paths.

        fn collect_themes(
            name: &OsStr,
            locations: &IconLocations,
            themes: &mut HashMap<OsString, Option<ThemeDescriptor>>,
        ) {
            // Skip if we already have this theme.
            if themes.contains_key(name) {
                return;
            }

            let descriptor = match locations.theme_description(name) {
                Ok(d) => Some(d),
                Err(_e) => {
                    #[cfg(feature = "log")]
                    log::debug!("skipping theme candidate {name:?} because {_e}");

                    None
                }
            };
            let descriptor = themes.entry(name.to_os_string()).insert_entry(descriptor);

            let Some(descriptor) = descriptor.get() else {
                return;
            };

            let parents = descriptor.index.inherits.clone();

            // Collect all parents of this theme:
            for parent in parents {
                collect_themes(parent.as_ref().as_ref(), locations, themes);
            }
        }

        // Map from theme names to their descriptor:
        let mut themes = HashMap::new();

        // collect all required themes:
        for theme_name in theme_names {
            let theme_name = theme_name.as_ref();
            collect_themes(theme_name.as_ref(), &self, &mut themes);
        }

        // make 100% sure we have `hicolor`, for the half-impossible edge-case of only collecting
        // themes that does not have hicolor in their inheritance tree
        collect_themes("hicolor".as_ref(), &self, &mut themes);
        // of course, the user might be cursed and not have `hicolor` installed at all!
        // that is troubling, but we'll see that it is handled correctly below.

        // let's prune theme candidates that have no description (meaning they weren't themes, or
        //  were invalid)
        // we'll also split them up, as `theme_chains` borrows names from `theme_names`,
        // but we need to mutate theme_descriptions later (during the borrow) to avoid
        // cloning the descriptions
        let (theme_names, mut theme_descriptions): (Vec<_>, Vec<_>) = themes
            .into_iter()
            .flat_map(|(key, value)| value.map(|v| (key, Some(v))))
            .unzip();

        // the Options are there just so we can take descriptions out of the vec without messing up the order.
        debug_assert!(theme_descriptions.iter().all(Option::is_some));

        // do we even have hicolor?
        // if not, there's no use in inserting hicolor into the inheritance tree later
        let hicolor_idx = theme_names.iter().position(|name| name == "hicolor");

        // Time to find the optimal ancestry for each theme.
        // as hicolor _should_ have all icons by default, and all themes depend on hicolor at some depth,
        // DFS would de facto end up in hicolor before ever trying the second theme in an Inherits set.
        // therefore BFS is the only sensible option, but the spec doesn't define this.

        // indexed by the position in our theme_names/theme_descriptions vecs
        let number_of_themes = theme_names.len();
        let mut theme_chains = Vec::<Vec<usize>>::with_capacity(number_of_themes);

        for theme_idx in 0..number_of_themes {
            let mut chain = Vec::from([theme_idx]);

            let mut cursor = 0;
            while let Some(node_idx) = chain.get(cursor).copied() {
                cursor += 1;

                let Some(Some(description)) = theme_descriptions.get(node_idx) else {
                    continue;
                };

                for parent in &description.index.inherits {
                    let Some(parent_idx) = theme_names
                        .iter()
                        .position(|name| *name.as_os_str() == **parent)
                    else {
                        // this parent was invalid
                        continue;
                    };

                    if !chain.contains(&parent_idx) {
                        chain.push(parent_idx);
                    }
                }
            }

            // From the spec: "If no theme is specified, implementations are required to add the
            //                 "hicolor" theme to the inheritance tree."
            if let Some(hicolor_idx) = hicolor_idx {
                if !chain.contains(&hicolor_idx) {
                    chain.push(hicolor_idx);
                }
            }

            theme_chains.push(chain);
        }

        // at this point `theme_chains` contains a _topological order_ for each theme's parents,
        // meaning we can easily iterate over it, constructing `Theme`s, assuming at every point
        // that each parent already has a `Theme` created for it :)

        // again indexed by theme indices, None values mean the theme hasn't been processed yet.
        // the goal is that, by the end of the for loop, that this only contains `Some`s.
        // we rely on the topological order of chains to always have all the prerequisite themes
        // present already in this map!
        let mut full_themes = vec![None::<Arc<Theme>>; number_of_themes];

        for chain in &theme_chains {
            // go from last theme to first, as all dependencies are "forward" in the chain:
            for theme_idx in chain.iter().copied().rev() {
                let theme_desc = theme_descriptions[theme_idx].take();

                let Some(theme_desc) = theme_desc else {
                    // the option was None, meaning this theme was processed already :-)
                    continue;
                };

                let parents = &theme_chains[theme_idx];
                let parents = parents
                    .into_iter()
                    .skip(1) // the first in the chain is the theme itself, which we'll ignore—it's not a parent.
                    .copied()
                    // unwrap OK because, by the topological order, all of these parents
                    // should already be present in the array:
                    .map(|parent_idx| Arc::clone(full_themes[parent_idx].as_ref().unwrap()))
                    .collect();

                let theme = Theme {
                    description: theme_desc,
                    parents,
                };

                full_themes[theme_idx] = Some(Arc::new(theme));
            }
        }

        debug_assert!(full_themes.iter().all(Option::is_some));

        let full_themes: Vec<_> = full_themes.into_iter().map(Option::unwrap).collect();

        // and so, we have reached the end of the Big Beautiful Function.
        // `full_themes` is a list of
        // - All themes requested,
        // - all themes required by the inheritance tree of those themes, without duplicates,
        // - and an optimal chain (inheritance tree search order) for each theme.

        full_themes
    }

    pub fn theme_description<S>(&self, internal_name: S) -> std::io::Result<OwnedThemeDescriptor>
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

        self.standalone_icons
            .iter()
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

        let descriptor = locations.theme_description("Adwaita").unwrap();
        assert_eq!(descriptor.index.name, "Adwaita");

        let icon = locations.standalone_icon("htop").unwrap();
        assert_eq!(icon.path.file_name(), Some("htop.png".as_ref()))
    }

    #[test]
    fn test_2() {
        let result = SearchDirectories::default()
            .find_icon_locations()
            .theme_description("breeze")
            .unwrap();

        println!("{:?}", result.index.inherits);
    }

    #[test]
    fn test() {
        let _dirs = SearchDirectories::default().find_icon_locations().resolve();

        // it didn't panic.
    }
}
