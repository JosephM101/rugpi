//! In-memory representation of Rugpi Bakery projects.

use std::{
    cell::OnceCell,
    path::{Path, PathBuf},
    sync::Arc,
};

use rugpi_common::Anyhow;

use self::{config::BakeryConfig, library::Library, repositories::ProjectRepositories};

pub mod config;
pub mod images;
pub mod layers;
pub mod library;
pub mod recipes;
pub mod repositories;

/// Extension trait for [`OnceCell`].
pub trait OnceCellExt<T> {
    /// Gets the contents of the cell or tries to initialize it.
    ///
    /// We can remove this once `get_or_try_init` lands in the standard library (see
    /// [#109737](https://github.com/rust-lang/rust/issues/109737)).
    fn try_get_or_init<E>(&self, init: impl FnOnce() -> Result<T, E>) -> Result<&T, E>;
}

impl<T> OnceCellExt<T> for OnceCell<T> {
    fn try_get_or_init<E>(&self, init: impl FnOnce() -> Result<T, E>) -> Result<&T, E> {
        if let Some(value) = self.get() {
            return Ok(value);
        }
        self.set(init()?).ok();
        Ok(self.get().unwrap())
    }
}

/// A project.
#[derive(Debug)]
#[non_exhaustive]
pub struct Project {
    /// The configuration of the project.
    pub config: BakeryConfig,
    /// The project directory.
    pub dir: PathBuf,
    /// Lazily initialized fields.
    lazy: ProjectLazy,
}

impl Project {
    /// The repositories of the project.
    pub fn repositories(&self) -> Anyhow<&Arc<ProjectRepositories>> {
        self.lazy
            .repositories
            .try_get_or_init(|| ProjectRepositories::load(self).map(Arc::new))
    }

    /// The library of the project.
    pub fn library(&self) -> Anyhow<&Arc<Library>> {
        self.lazy.library.try_get_or_init(|| {
            let repositories = self.repositories()?.clone();
            Library::load(repositories).map(Arc::new)
        })
    }
}

/// Lazily initialized fields of [`Project`].
#[derive(Debug, Default)]
struct ProjectLazy {
    /// The repositories of the project.
    repositories: OnceCell<Arc<ProjectRepositories>>,
    /// The library of the project.
    library: OnceCell<Arc<Library>>,
}

/// Project loader.
#[derive(Debug)]
pub struct ProjectLoader {
    /// The project directory.
    project_dir: PathBuf,
    /// Path to the configuration file.
    config_file: Option<PathBuf>,
}

impl ProjectLoader {
    /// Construct a new project loader with the given project directory.
    pub fn new(project_dir: &Path) -> Self {
        Self {
            project_dir: project_dir.to_path_buf(),
            config_file: None,
        }
    }

    /// Construct a new project loader from the current working directory.
    pub fn current_dir() -> Anyhow<Self> {
        Ok(Self::new(&std::env::current_dir()?))
    }

    /// Set the configuration file path relative to the project directory.
    pub fn with_config_file(mut self, config_file: Option<&Path>) -> Self {
        self.config_file = config_file.map(Path::to_path_buf);
        self
    }

    /// The full path to the configuration file.
    fn config_path(&self) -> PathBuf {
        self.project_dir.join(
            self.config_file
                .as_deref()
                .unwrap_or_else(|| Path::new("rugpi-bakery.toml")),
        )
    }

    /// Load the project.
    pub fn load(self) -> Anyhow<Project> {
        let config = BakeryConfig::load(&self.config_path())?;
        Ok(Project {
            dir: self.project_dir,
            config,
            lazy: ProjectLazy::default(),
        })
    }
}
