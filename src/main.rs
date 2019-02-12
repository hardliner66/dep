#[macro_use]
extern crate serde_derive;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::exit;

use argparse::ArgumentParser;
use argparse::Store;
use git2;
use git2::build::CheckoutBuilder;
use git2::build::RepoBuilder;
use git2::FetchOptions;
use git2::RemoteCallbacks;

#[cfg(windows)]
mod systools {
    use std::env::VarError;
    use std::os::windows::fs::symlink_dir;
    use std::path::Path;

    pub fn make_symlink<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dst: Q) -> Result<(), std::io::Error> {
        symlink_dir(src, dst)
    }

    pub fn get_home_dir_env_var() -> &'static str {
        "USERPROFILE"
    }

    pub fn get_home_dir() -> Result<String, VarError> {
        std::env::var(get_home_dir_env_var())
    }
}

#[cfg(unix)]
mod systools {
    use std::env::VarError;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    pub fn make_symlink<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dst: Q) -> Result<(), std::io::Error> {
        symlink(src, dst)
    }

    pub fn get_home_dir_env_var() -> &'static str {
        "HOME"
    }

    pub fn get_home_dir() -> Result<String, VarError> {
        std::env::var(get_home_dir_env_var())
    }
}

fn normalize<P>(path: &P) -> PathBuf
    where P: AsRef<Path>
{
    let path_string = path.as_ref().to_string_lossy().to_string();
    let split_char = if path_string.contains("/") {
        "/"
    } else {
        "\\"
    };

    let parts = path_string.split(split_char);

    let mut result = PathBuf::new();

    for part in parts {
        if part.starts_with("$") {
            let var = &part[1..];
            result.push(&std::env::var(var).unwrap());
        } else if part.starts_with("%") && part.ends_with("%") {
            let var = &part[1..part.len() - 1];
            result.push(&std::env::var(var).unwrap());
        } else if part == "~" {
            result.push(&std::env::var(systools::get_home_dir_env_var()).unwrap());
        } else {
            result.push(part);
        }
    }

    result
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct SshOptions {
    private: PathBuf,
    public: PathBuf,
    protected: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct GeneralOptions {
    default_lib_dir: PathBuf,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct GlobalOptions {
    general: GeneralOptions,
    ssh: Option<SshOptions>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub struct TomlDependency {
    path: Option<PathBuf>,
    repo: Option<String>,
    git: Option<String>,
    branch: Option<String>,
    tag: Option<String>,
    rev: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct TomlProject {
    name: String,
    lib_dir: Option<PathBuf>,
    git_server: Option<String>,

    // package metadata
    authors: Option<Vec<String>>,
    description: Option<String>,
    homepage: Option<String>,
    repository: Option<String>,
    metadata: Option<toml::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct TomlManifest {
    project: TomlProject,
    dependencies: Option<BTreeMap<String, TomlDependency>>,
}

fn read(file: &mut File) -> std::result::Result<String, std::io::Error> {
    let mut content = String::new();
    match file.read_to_string(&mut content) {
        Ok(_) => Ok(content),
        Err(e) => Err(e),
    }
}

#[derive(Debug)]
struct Options {
    command: String,
}

fn get_options() -> Options {
    let mut command = "".to_string();
    {
        // this block limits scope of borrows by ap.refer() method
        let mut ap = ArgumentParser::new();
        ap.set_description("Dependency manager.");
        ap.refer(&mut command)
            .add_argument("command", Store, "the command to execute. [init, update]");
        ap.parse_args_or_exit();
    }
    Options {
        command: command.to_lowercase().trim().to_string(),
    }
}

static mut GLOBAL_OPTIONS: Option<GlobalOptions> = None;

fn set_global_options(opts: &GlobalOptions) {
    unsafe {
        GLOBAL_OPTIONS = Some(opts.clone());
    }
}

fn get_global_options() -> GlobalOptions {
    unsafe {
        match &GLOBAL_OPTIONS {
            Some(opts) => opts.clone(),
            None => GlobalOptions {
                ssh: Some(SshOptions {
                    private: Path::new(&format!("${}/.ssh/id_rsa", systools::get_home_dir_env_var())).to_path_buf(),
                    public: Path::new(&format!("${}/.ssh/id_rsa.pub", systools::get_home_dir_env_var())).to_path_buf(),
                    protected: false,
                }),
                general: GeneralOptions {
                    default_lib_dir: Path::new("VENDOR").to_path_buf()
                },
            },
        }
    }
}

static mut PASSPHRASE: Option<String> = None;

fn set_passphrase(str: &String) {
    unsafe {
        PASSPHRASE = Some(str.clone());
    }
}

fn get_passphrase() -> String {
    unsafe {
        match &PASSPHRASE {
            Some(s) => s.clone(),
            None => "".to_owned(),
        }
    }
}

fn main() -> std::result::Result<(), Box<std::error::Error>> {
    match systools::get_home_dir() {
        Ok(dir) => {
            let global_config_path = Path::new(&dir).join(".deprc");
            if !global_config_path.exists() {
                let opts = get_global_options();

                let mut file = File::create(&global_config_path)?;
                let val = toml::ser::to_string_pretty(&opts)?;

                file.write_all(val.as_bytes())?;
                file.flush()?;
                println!("Initializing global configuration.");
                set_global_options(&opts);
            } else {
                let mut file = File::open(&global_config_path)?;

                let config = read(&mut file)?;

                set_global_options(&toml::de::from_str(&config)?);
            }
        }
        _ => {
            eprintln!("Could not get homedir, using default global config");
        }
    };

    let file_path = Path::new("./deps.toml");

    let opts = get_global_options();

    let options = get_options();
    if options.command == "global" {
        match systools::get_home_dir() {
            Ok(dir) => {
                let global_config_path = Path::new(&dir).join(".deprc");
                println!("Global configuration path: \"{}\"", global_config_path.to_string_lossy());
            }
            _ => {
                eprintln!("Could not get homedir, using default global config");
            }
        };
    } else if options.command == "init" {
        if Path::exists(file_path) {
            eprintln!("Already initialized");
            exit(1);
        }

        let man = TomlManifest {
            project: TomlProject {
                name: std::env::current_dir()?.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or("".to_owned()),
                authors: Some(vec![whoami::username()]),
                lib_dir: None,
                git_server: None,

                // package metadata
                description: None,
                homepage: None,
                repository: None,
                metadata: None,
            },
            dependencies: None,
        };

        let mut file = File::create(&file_path)?;
        let val = toml::ser::to_string_pretty(&man)?;

        file.write_all(val.as_bytes())?;
        file.flush()?;
    } else if options.command == "update" {
        let mut file = File::open(&file_path)?;

        let config = read(&mut file)?;

        let man: TomlManifest = toml::de::from_str(&config)?;

        let libdir = match &man.project.lib_dir {
            Some(dir) => dir.clone(),
            None => opts.general.default_lib_dir.clone(),
        };
        if !libdir.exists() {
            println!("Creating lib dir: {}", libdir.to_string_lossy());
            std::fs::create_dir_all(&libdir)?;
//        } else {
//            println!("Deleting old lib dir: {}", libdir.to_string_lossy());
//            remove_dir_all::remove_dir_all(&libdir)?;
//            println!("Creating lib dir: {}", libdir.to_string_lossy());
//            std::fs::create_dir_all(&libdir)?;
        }

        match &man.dependencies {
            None => (),
            Some(deps) => {
                for lib in libdir.read_dir()?.filter(|p| {
                    match p {
                        Ok(d) => {
                            !deps.keys().any(|k| *k == d.file_name().to_string_lossy().to_string())
                        }
                        Err(_) => false,
                    }
                }).flat_map(|d| match d {
                    Ok(d) => Some(d),
                    _ => None,
                }) {
                    remove_dir_all::remove_dir_all(&lib.path())?;
                }
                if deps.values().any(|d| d.git.is_some() || (d.repo.is_some() && man.project.git_server.is_some())) {
                    match opts.ssh {
                        Some(ssh) => {
                            if ssh.protected {
                                match read_password() {
                                    Ok(pass) => set_passphrase(&pass.clone()),
                                    Err(e) => return Err(Box::new(e)),
                                };
                            }
                        }
                        _ => (),
                    }
                }


                for (name, dep) in deps {
                    let dst = libdir.join(Path::new(&name));

                    match &dep.path {
                        Some(path) => {
                            if !dst.exists() {
                                println!("Linking path: {}", path.to_string_lossy());
                                systools::make_symlink(&path, &dst)?;
                            }
                        }
                        None => {
                            let url = match (&man.project.git_server, &dep.repo, &dep.git) {
                                (Some(server), Some(repo), None) => if !server.contains("@") {
                                    if server.contains("://") {
                                        let mut parts = server.split("://");
                                        match (parts.nth(0), parts.nth(1)) {
                                            (Some(protocol), Some(server)) => {
                                                format!("{}://git@{}:{}", protocol, server, repo)
                                            }
                                            _ => unreachable!(),
                                        }
                                    } else {
                                        format!("git@{}:{}", server, repo)
                                    }
                                } else {
                                    format!("{}:{}", server, repo)
                                },
                                (None, None, Some(repo)) => repo.clone(),
                                (Some(_), None, Some(repo)) => repo.clone(),
                                _ => return Err(Box::new(git2::Error::from_str("Could not get git url or dependency path"))),
                            };

                            let mut cb = RemoteCallbacks::new();
                            cb.credentials(credentials);

                            let mut fo = FetchOptions::new();
                            fo.remote_callbacks(cb);

                            let co = CheckoutBuilder::new();

                            match (&dep.branch, &dep.tag, &dep.rev) {
                                (Some(branch_name), None, None) => {
                                    println!("Cloning branch \"{}\" from {}", branch_name, url);
                                    if !dst.exists() {
                                        std::fs::create_dir_all(&dst)?;
                                        RepoBuilder::new().branch(branch_name).fetch_options(fo).with_checkout(co)
                                            .clone(&url, Path::new(&dst))?;
                                    } else {
                                        let repo = git2::Repository::open(&dst)?;

                                        let mut remote = repo.find_remote("origin")?;

                                        let mut cb = RemoteCallbacks::new();
                                        cb.credentials(credentials);

                                        remote.connect_auth(git2::Direction::Fetch, Some(cb), None)?;

                                        let mut cb = RemoteCallbacks::new();
                                        cb.credentials(credentials);

                                        let mut fo = FetchOptions::new();
                                        fo.remote_callbacks(cb);

                                        let mut co = CheckoutBuilder::new();
                                        co.refresh(true);
                                        co.recreate_missing(true);
                                        co.update_index(true);
                                        co.overwrite_ignored(true);
                                        co.allow_conflicts(false);

                                        let local_branch_name = format!("refs/heads/{}", branch_name);

                                        let local_branch = repo.find_branch(&branch_name, git2::BranchType::Local)?;
                                        let local_branch_ref = local_branch.into_reference();
                                        let local_branch_tree = local_branch_ref.peel_to_tree()?;

                                        let local_branch = local_branch_tree.as_object();

                                        remote.fetch(&["refs/heads/*:refs/heads/*"], Some(&mut fo), None)?;
                                        remote.download(&["refs/heads/*:refs/heads/*"], Some(&mut fo))?;

                                        remote.disconnect();

                                        repo.set_head(&local_branch_name)?;

                                        repo.checkout_tree(&local_branch, Some(&mut co))?;

                                        repo.reset(repo.head()?.peel_to_commit()?.as_object(), git2::ResetType::Mixed, None)?;

                                        repo.cleanup_state()?;
                                    }
                                }
                                (None, Some(tag), None) => {
                                    println!("Cloning tag \"{}\" from {}", tag, url);
                                    let repo = if !dst.exists() {
                                        std::fs::create_dir_all(&dst)?;
                                        RepoBuilder::new().fetch_options(fo).with_checkout(co)
                                            .clone(&url, Path::new(&dst))?
                                    } else {
                                        git2::Repository::open(&dst)?
                                    };
                                    let mut remote = repo.find_remote("origin")?;

                                    let full_tag = format!("refs/tags/{}", tag);

                                    let mut cb = RemoteCallbacks::new();
                                    cb.credentials(credentials);

                                    let mut fo = FetchOptions::new();
                                    fo.remote_callbacks(cb);

                                    let mut co = CheckoutBuilder::new();

                                    remote.download(&[&full_tag], Some(&mut fo))?;

                                    repo.checkout_tree(repo.find_reference(&full_tag)?.peel_to_tag()?.as_object(), Some(&mut co))?;

                                    repo.set_head(&full_tag)?;
                                }
                                (None, None, Some(rev)) => {
                                    println!("Cloning revision \"{}\" from {}", rev, url);
                                    let repo = if !dst.exists() {
                                        std::fs::create_dir_all(&dst)?;
                                        RepoBuilder::new().fetch_options(fo).with_checkout(co)
                                            .clone(&url, Path::new(&dst))?
                                    } else {
                                        git2::Repository::open(&dst)?
                                    };

                                    let mut cb = RemoteCallbacks::new();
                                    cb.credentials(credentials);

                                    let mut fo = FetchOptions::new();
                                    fo.remote_callbacks(cb);

                                    let mut co = CheckoutBuilder::new();

                                    let commit = &repo.find_commit(git2::Oid::from_str(&rev)?)?;

                                    repo.checkout_tree(&commit.as_object(), Some(&mut co))?;

                                    repo.set_head_detached(commit.id())?;
                                }
                                _ => {
                                    println!("Cloning repository from {}", url);
                                    if !dst.exists() {
                                        std::fs::create_dir_all(&dst)?;
                                        RepoBuilder::new().fetch_options(fo).with_checkout(co)
                                            .clone(&url, Path::new(&dst))?;
                                    } else {
                                        let repo = git2::Repository::open(&dst)?;
                                        let mut remote = repo.find_remote("origin")?;

                                        let mut cb = RemoteCallbacks::new();
                                        cb.credentials(credentials);

                                        let mut fo = FetchOptions::new();
                                        fo.remote_callbacks(cb);

                                        let mut co = CheckoutBuilder::new();

                                        remote.download(&[], Some(&mut fo))?;

                                        repo.checkout_head(Some(&mut co))?;
                                    }
                                }
                            };
                        }
                    }
                }
            }
        }
    } else {
        eprintln!("Unknown command: \"{}\"", options.command);
        exit(2);
    }

    Ok(())
}

fn read_password() -> Result<String, std::io::Error> {
    let pass = rpassword::prompt_password_stderr("Enter Passphrase: ");
    println!();
    pass
}


pub fn credentials(
    _user: &str,
    user_from_url: Option<&str>,
    _cred: git2::CredentialType,
) -> Result<git2::Cred, git2::Error> {
    let opts = get_global_options();
    match opts.ssh {
        Some(ssh) => {
            let id_rsa_pub = Path::new(&ssh.public);

            match user_from_url {
                Some(user) => git2::Cred::ssh_key(user, Some(&normalize(&id_rsa_pub)), &normalize(&ssh.private), Some(get_passphrase().as_str())),
                None => Err(git2::Error::from_str("Url does not contain username")),
            }
        }
        None => {
            match systools::get_home_dir() {
                Ok(p) => {
                    let base = Path::new(&p).join(".ssh");
                    let id_rsa = base.join("id_rsa");
                    let id_rsa_pub = base.join("id_rsa.pub");
                    match user_from_url {
                        Some(user) => git2::Cred::ssh_key(user, Some(&id_rsa_pub), &id_rsa, None),
                        None => Err(git2::Error::from_str("Url does not contain username")),
                    }
                }
                _ => Err(git2::Error::from_str("USERPROFILE not set")),
            }
        }
    }
}