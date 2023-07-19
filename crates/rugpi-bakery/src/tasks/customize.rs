//! Applies a set of recipes to a system.

use std::{
    collections::{HashMap, HashSet},
    env, fs,
    ops::Deref,
    path::PathBuf,
    sync::Arc,
};

use anyhow::bail;
use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser;
use tempdir::TempDir;
use xscript::{cmd, run, vars, ParentEnv, Run};

use crate::{
    config::BakeryConfig,
    recipes::{Recipe, RecipeLibrary, StepKind},
    utils::Mounted,
};

/// The arguments of the `customize` command.
#[derive(Debug, Parser)]
pub struct CustomizeTask {
    /// The source archive with the original system.
    src: String,
    /// The destination archive with the modified system.
    dest: String,
}

pub fn run(task: &CustomizeTask) -> anyhow::Result<()> {
    // 1️⃣ Load the Bakery configuration file.
    let current_dir = Utf8PathBuf::try_from(env::current_dir()?)?;
    let config = toml::from_str(&fs::read_to_string(current_dir.join("rugpi-bakery.toml"))?)?;
    // 2️⃣ Collect the recipes to apply.
    let jobs = recipe_schedule(&config)?;
    // 3️⃣ Prepare system chroot.
    let root_dir = TempDir::new("rugpi")?;
    let root_dir_path = Utf8Path::from_path(root_dir.path()).unwrap();
    println!("Extracting system files...");
    run!(["tar", "-x", "-f", &task.src, "-C", root_dir_path])?;
    apply_recipes(&jobs, root_dir_path)?;
    println!("Packing system files...");
    run!(["tar", "-c", "-f", &task.dest, "-C", root_dir_path, "."])?;
    Ok(())
}

struct RecipeJob {
    recipe: Arc<Recipe>,
    parameters: HashMap<String, String>,
}

fn recipe_schedule(config: &BakeryConfig) -> anyhow::Result<Vec<RecipeJob>> {
    let mut library = RecipeLibrary::new();
    // 1️⃣ Load builtin recipes.
    let builtin_recipes_path = PathBuf::from(
        env::var("RUGPI_BUILTIN_RECIPES_PATH")
            .unwrap_or_else(|_| "/usr/share/rugpi/recipes".to_owned()),
    );
    library.loader().load_all(&builtin_recipes_path)?;
    // 2️⃣ Load custom recipes.
    let custom_recipes_path = env::current_dir()?.join("recipes");
    if custom_recipes_path.is_dir() {
        library
            .loader()
            .with_default(true)
            .load_all(&custom_recipes_path)?;
    }
    // 3️⃣ Collect the recipes to apply. This is certainly not the most efficient
    // implementation, but for our purposes it should suffice.
    let mut stack = (&library)
        .into_iter()
        .filter_map(|(name, recipe)| {
            let is_default = recipe.info.default.unwrap_or_default();
            let is_excluded = config.exclude.contains(name);
            if is_default && !is_excluded {
                Some(name)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    stack.extend(&config.recipes);
    let mut recipe_names = stack.iter().cloned().collect::<HashSet<_>>();
    while let Some(name) = stack.pop() {
        for name in &library.get(name)?.info.dependencies {
            if recipe_names.insert(name) {
                stack.push(name);
            }
        }
    }
    let mut recipes = recipe_names
        .into_iter()
        .map(|name| {
            let recipe = library.get(name).unwrap().clone();
            let recipe_params = config.parameters.get(name);
            let mut parameters = HashMap::new();
            for (name, def) in &recipe.info.parameters {
                if let Some(params) = recipe_params {
                    if let Some(value) = params.get(name) {
                        parameters.insert(name.to_owned(), value.to_string());
                        continue;
                    }
                }
                if let Some(default) = &def.default {
                    parameters.insert(name.to_owned(), default.to_string());
                    continue;
                }
                bail!("unable to find value for parameter `{name}`");
            }
            Ok(RecipeJob { recipe, parameters })
        })
        .collect::<Result<Vec<_>, _>>()?;
    // 4️⃣ Sort recipes by priority.
    recipes.sort_by_key(|job| -job.recipe.info.priority);
    Ok(recipes)
}

fn apply_recipes(jobs: &Vec<RecipeJob>, root_dir_path: &Utf8Path) -> anyhow::Result<()> {
    let _mounted_dev = Mounted::bind("/dev", root_dir_path.join("dev"))?;
    let _mounted_dev_pts = Mounted::bind("/dev/pts", root_dir_path.join("dev/pts"))?;
    let _mounted_sys = Mounted::bind("/sys", root_dir_path.join("sys"))?;
    let _mounted_proc = Mounted::mount_fs("proc", "proc", root_dir_path.join("proc"))?;
    let _mounted_run = Mounted::mount_fs("tmpfs", "tmpfs", root_dir_path.join("run"))?;
    let _mounted_tmp = Mounted::mount_fs("tmpfs", "tmpfs", root_dir_path.join("tmp"))?;

    let bakery_recipe_path = root_dir_path.join("run/rugpi/bakery/recipe");
    fs::create_dir_all(&bakery_recipe_path)?;

    for (idx, job) in jobs.iter().enumerate() {
        let recipe = &job.recipe;
        println!(
            "[{:>2}/{}] {} {:?}",
            idx + 1,
            jobs.len(),
            recipe
                .info
                .description
                .as_deref()
                .unwrap_or(recipe.name.deref()),
            &job.parameters,
        );
        let _mounted_recipe = Mounted::bind(&recipe.path, &bakery_recipe_path)?;

        for step in &recipe.steps {
            println!("    - {}", step.filename);
            match &step.kind {
                StepKind::Packages { packages } => {
                    let mut cmd = cmd!("chroot", root_dir_path, "apt-get", "install", "-y");
                    cmd.extend_args(packages);
                    ParentEnv.run(&cmd.with_vars(vars! {
                        DEBIAN_FRONTEND = "noninteractive"
                    }))?;
                }
                StepKind::Install => {
                    let script = format!("/run/rugpi/bakery/recipe/steps/{}", step.filename);
                    let mut vars = vars! {
                        DEBIAN_FRONTEND = "noninteractive",
                        RUGPI_ROOT_DIR = "/",
                        RECIPE_DIR = "/run/rugpi/bakery/recipe/",
                        RECIPE_STEP_PATH = &script,
                    };
                    for (name, value) in &job.parameters {
                        vars.set(format!("RECIPE_PARAM_{}", name.to_uppercase()), value);
                    }
                    run!(["chroot", root_dir_path, &script].with_vars(vars))?;
                }
                StepKind::Run => {
                    let script = recipe.path.join("steps").join(&step.filename);
                    let mut vars = vars! {
                        DEBIAN_FRONTEND = "noninteractive",
                        RUGPI_ROOT_DIR = root_dir_path,
                        RECIPE_DIR = &recipe.path,
                        RECIPE_STEP_PATH = &script,
                    };
                    for (name, value) in &job.parameters {
                        vars.set(format!("RECIPE_PARAM_{}", name.to_uppercase()), value);
                    }
                    run!([&script].with_vars(vars))?;
                }
            }
        }
    }
    Ok(())
}