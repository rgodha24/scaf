use clap::Parser;
use dialoguer::MultiSelect;
use regex::Regex;
use serde::Deserialize;
use std::{
    cell::OnceCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process,
    sync::OnceLock,
};

fn main() {
    let args = Args::parse();
    let path = Path::new(&args.template_path);

    if !path.exists() {
        eprintln!("Path does not exist: {}", path.display());
        process::exit(1);
    }

    let config = Config::from_base(&args.template_path);

    let chosen = select_options(&config);
    let files = read_files_from_path(path);

    let files = dedupe_files(files, &chosen);

    match std::fs::read_dir(&args.output_path) {
        Ok(d) => {
            if d.into_iter().count() != 0 {
                eprintln!("Error: output directory is not empty");
                process::exit(1);
            }
        }
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => {
                std::fs::create_dir_all(&args.output_path).expect("valid path");
            }
            _ => {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        },
    }

    let mut files = files.into_iter().collect::<Vec<_>>();

    replace_file_paths(&mut files, &args);

    for f in &files {
        println!("path: {}, options: {:?}", f.path.display(), f.depends_on);
    }

    write_files(files);
}

fn write_files(files: Vec<File>) {
    for f in files {
        std::fs::write(f.path, f.contents).expect("valid path");
    }
}

fn replace_file_paths(files: &mut Vec<File>, args: &Args) {
    for f in files {
        let stripped = f
            .path
            .strip_prefix(args.template_path.clone())
            .expect("prefix is the same");

        f.path = args.output_path.join(stripped);
    }
}

fn read_files_from_path(path: &Path) -> Vec<File> {
    let mut f_vec = vec![];
    let files = std::fs::read_dir(path).unwrap();
    for file in files.into_iter().filter_map(|f| match f {
        Ok(f) => Some(f),
        Err(e) => {
            eprintln!("Error reading a file: {}", e);
            None
        }
    }) {
        // pain
        let path = String::from(file.path().as_os_str().to_str().expect("normal string"));

        if path.contains("scaf.toml") {
            continue;
        }

        let options = options_in_file(&path);

        f_vec.push(create_file(path, options));
    }
    f_vec
}

// TODO: not OnceLock ??
static RE: OnceLock<Regex> = OnceLock::new();

fn options_in_file(path: &String) -> Vec<String> {
    let re = RE.get_or_init(|| Regex::new(r"\{.+\}").expect("valid regex"));

    let caps = match re.captures(&path) {
        None => vec![],
        Some(c) => c
            .iter()
            .filter_map(|i| i.map(|i| i.as_str().trim_matches(|c| c == '{' || c == '}')))
            .map(String::from)
            .collect(),
    };

    let mut options = HashSet::new();

    for c in caps {
        for o in c.split(',') {
            options.insert(String::from(o));
        }
    }

    options.into_iter().collect()
}

fn create_file(path: String, options: Vec<String>) -> File {
    let re = RE.get_or_init(|| Regex::new(r"\{.+\}").expect("valid regex"));
    let contents = std::fs::read_to_string(&path).expect("valid utf8");

    let path = re.replace_all(&path, "");
    let path = PathBuf::from(path.into_owned());

    File {
        path,
        contents,
        depends_on: options,
    }
}

fn select_options(config: &Config) -> Vec<String> {
    let mut map = config.options.iter().collect::<Vec<_>>();
    map.sort_by(|(_, a), (_, b)| a.cmp(b));
    let items = map.iter().map(|(_, v)| v).collect::<Vec<_>>();

    // TODO: instructions
    let chosen = MultiSelect::new().items(&items).interact().unwrap();
    let chosen = chosen.iter().map(|&i| map[i].0.clone()).collect::<Vec<_>>();

    println!("{:?}", chosen);

    chosen
}

fn dedupe_files(files: Vec<File>, chosen: &Vec<String>) -> HashSet<File> {
    let files: Vec<_> = files
        .into_iter()
        // first filter out all the ones that don't depend on any of the chosen options
        .filter(|f| f.depends_on.iter().all(|o| chosen.contains(o)))
        .collect();

    let mut deduped_files = HashSet::new();
    // now we check for duplicates in the paths and try to resolve them
    for f in &files {
        // definitely a better way of doing this i just dont know it
        let indexes = files
            .iter()
            .enumerate()
            .filter_map(|(i, f2)| if f.path == f2.path { Some(i) } else { None })
            .collect::<Vec<_>>();

        if indexes.len() > 1 {
            let dups = indexes
                .into_iter()
                .map(|i| files[i].clone())
                .collect::<Vec<_>>();

            let max = dups.iter().map(|a| a.depends_on.len()).max().unwrap();
            let maxes = dups
                .iter()
                .filter(|i| i.depends_on.len() == max)
                .collect::<Vec<_>>();
            if maxes.len() > 1 {
                eprintln!(
                    "Error: can't choose between files with similar options. filename: {}",
                    maxes[0].path.display()
                );
                process::exit(1);
            } else {
                deduped_files.insert(maxes[0].clone());
            }
        } else {
            deduped_files.insert(f.clone());
        }
    }

    deduped_files
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct File {
    path: PathBuf,
    contents: String,
    depends_on: Vec<String>,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg()]
    template_path: PathBuf,
    #[arg()]
    output_path: PathBuf,
}

#[derive(Deserialize, Debug)]
struct Config {
    /// the options that scaf should give the user, key: variable name, value: human readable name
    options: HashMap<String, String>,
}

impl Config {
    fn from_base(base_path: &PathBuf) -> Self {
        let config_file_path = base_path.join("scaf.toml");
        let config_file = match std::fs::read(config_file_path.clone()) {
            Ok(file) => String::from_utf8(file).expect("valid utf8"),
            Err(e) => {
                eprintln!("Error reading {:?}: {}", config_file_path, e);
                process::exit(1);
            }
        };

        match toml::from_str::<Config>(&config_file) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Error parsing scaf.toml: {}", e);
                process::exit(1);
            }
        }
    }

    fn get_all_options(&self) -> Vec<String> {
        self.options.keys().cloned().collect::<Vec<_>>()
    }
}
