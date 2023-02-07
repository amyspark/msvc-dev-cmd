// SPDX-License-Ref: MPL-2.0

use anyhow::{Result, bail, Context};
use std::collections::{HashMap, HashSet};
use dunce::canonicalize;
use std::ffi::OsString;
use std::{env, process};
use std::io::Write;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, Arc};
use clap::Parser;
use log;
use env_logger;
use tempfile::Builder;

#[derive(Parser, Debug)]
#[command(version, author, about = "Run a command under your favourite Developer Shell Prompt", after_help = "Inspired by https://github.com/ilammy/msvc-dev-cmd")]
struct Opt {
    /// Target architecture
    #[arg(long, default_value = "x64")]
    arch: String,

    /// Windows SDK number to build for
    #[arg(long)]
    sdk: Option<String>,

    /// Enable Spectre mitigations
    #[arg(long, default_value_t = false)]
    spectre: bool,

    /// VC++ compiler toolset version
    #[arg(long)]
    toolset: Option<String>,

    /// Build for Universal Windows Platform
    #[arg(long, default_value_t = false)]
    uwp: bool,

    /// The Visual Studio version to use. This can be the version number (e.g. 16.0 for 2019) or the year (e.g. "2019").
    #[arg(long)]
    vsversion: Option<String>,

    /// Name or path to the program I'll background to.
    program: PathBuf,

    /// Arguments to the program.
    args: Vec<PathBuf>,
}

const EDITIONS: [&str; 5]  = ["Enterprise", "Professional", "Community", "Preview", "BuildTools"];

const YEARS: [&str; 4] = ["2022", "2019", "2017", "2015"];

const PATH_LIKE_VARIABLES: [&str; 4] = ["PATH", "INCLUDE", "LIB", "LIBPATH"];

fn pathbuf_from_key(key: &str) -> Result<PathBuf> {
    match env::var_os(key) {
        Some(v) => Ok(PathBuf::from(v)),
        None => bail!("The environment variable {} isn't set or is invalid", key),
    }
}

#[derive(Debug)]
struct Constants<'a>
{
    program_files_x86: PathBuf,
    program_files: Vec<PathBuf>,
    vs_year_version: HashMap<&'a str, &'a str>,
    vswhere_path: PathBuf,
}

impl Constants<'_> {
    pub fn new() -> Result<Constants<'static>> {
        let program_files_x86 = pathbuf_from_key("ProgramFiles(x86)")?;
        let program_files = pathbuf_from_key("ProgramFiles")?;
        Ok(Constants {
            program_files_x86: program_files_x86.clone(),
            program_files: vec![program_files_x86.clone(), program_files],
            vs_year_version: HashMap::from([
                ("2022", "17.0"),
                ("2019", "16.0"),
                ("2017", "15.0"),
                ("2015", "14.0"),
                ("2013", "12.0"),
            ]),
            vswhere_path: dunce::canonicalize(program_files_x86.join("Microsoft Visual Studio/Installer"))?
        })
    }

    fn vsversion_to_versionnumber(&self, vsversion: &Option<String>) -> Option<&str> {
        match vsversion  {
            Some(v) => self.vs_year_version.get(v.as_str()).copied(),
            None => None
        }
    }

    fn vsversion_to_year(&self, vsversion: &str) -> String {
        for (year, version) in self.vs_year_version.iter() {
            if vsversion.eq(version.deref()) {
                return year.to_string()
            }
        }

        String::from(vsversion)
    }

    fn find_with_vswhere(&self, pattern: &str, version_pattern: &str) -> Result<PathBuf> {
        let mut installation_path = {
            let mut cmd = Command::new("vswhere");

            cmd.args(["-products", "*"]).arg(version_pattern).arg("-prerelease").args(["-property", "installationPath"]).arg("-utf8");

            cmd
        };

        log::debug!("vswhere command: {:?} {:?}", installation_path.get_program(), installation_path.get_args());

        let output = installation_path.output()?;

        let path = String::from_utf8(output.stdout)?;

        log::debug!("vswhere output for query {}: {}", version_pattern, path.trim());

        if path.contains("Visual Studio Locator") || path.contains("Copyright (C)") {
            bail!("Query to vswhere failed:\n\t{}", path.trim());
        }

        let res = canonicalize( PathBuf::from(path.trim()).join(pattern))?;

        log::debug!("Result of vswhere query {}: {}", pattern, res.display());

        Ok(res)
    }

    fn find_vcvarsall(&self, vsversion: &Option<String>) -> Result<PathBuf> {
        let vsversion_number = self.vsversion_to_versionnumber(vsversion);
        let version_pattern = match vsversion_number {
            Some(v) => {
                let upper_bound = v.split(".").collect::<Vec<_>>()[0];
                format!("-version \"{},{}.9\"", v, upper_bound)
            },
            None => "-latest".to_string()
        };
    
        // If vswhere is available, ask it about the location of the latest Visual Studio.
        {
            let path = self.find_with_vswhere("VC/Auxiliary/Build/vcvarsall.bat", &version_pattern);
            match path {
                Ok(v) => {
                    log::info!("Found with vswhere: {}", v.display());
                    return Ok(v);
                },
                Err(v) => {
                    log::info!("Not found with vswhere: {}", v)
                }
            }
        }
    
        // If that does not work, try the standard installation locations,
        // starting with the latest and moving to the oldest.
        let years = match vsversion {
            Some(v) => vec![self.vsversion_to_year(&v)],
            None => YEARS.iter().map(|x| String::from(x.deref())).collect::<Vec<_>>()
        };

        for prog_files in self.program_files.iter() {
            for ver in years.iter() {
                for ed in EDITIONS {
                    let f = prog_files.join("Microsoft Visual Studio").join(ver).join(ed).join("VC/Auxiliary/Build/vcvarsall.bat");
                    log::info!("Trying standard location: {}", f.display());
                    let path = Path::new(&f);
                    if path.exists() {
                        log::info!("Found standard location: {}", f.display());
                        return Ok(f)
                    }
                }
            }
        }
        log::info!("Not found in standard locations");
    
        // Special case for Visual Studio 2015 (and maybe earlier), try it out too.
        let f = self.program_files_x86.join("Microsoft Visual C++ Build Tools/vcbuildtools.bat");
        let path = Path::new(&f);

        if path.exists() {
            log::info!("Found VS 2015: {}", f.display());
            return Ok(f)
        }
        
        log::info!("Not found in VS 2015 location: {}", f.display());

        bail!("Microsoft Visual Studio not found")
    }
}


fn is_path_variable(name: &str) -> bool {
    let key = name.to_uppercase();
    PATH_LIKE_VARIABLES.iter().any(|x| x.eq(&key))
}

/// Remove duplicates by keeping the first occurance and preserving order.
/// This keeps path shadowing working as intended.
fn filter_path_value(path: &str) -> String {
    return path.split(";").into_iter().collect::<HashSet<_>>().into_iter().collect::<Vec<_>>().join(";")
}

fn setup_msvcdev_cmd(opt: &Opt) -> Result<()> {
    let constants = Constants::new()?;

    if !cfg!(windows) {
        bail!("This is not a Windows virtual environment!")
    }

    // Add standard location of "vswhere" to PATH, in case it's not there.
    let path = match std::env::var_os("PATH") {
        Some(v) => v,
        None => OsString::new()
    };

    let extended_path = {
        let mut paths = env::split_paths(&path).into_iter().collect::<Vec<_>>();
    
        paths.push(constants.vswhere_path.clone());

        env::join_paths(paths.iter())?
    };

    log::debug!("Setting PATH to: {:?}", extended_path);

    env::set_var("PATH", extended_path);

    // There are all sorts of way the architectures are called. In addition to
    // values supported by Microsoft Visual C++, recognize some common aliases.
    let arch_aliases = HashMap::from([
        ("win32", "x86"),
        ("win64", "x64"),
        ("x86_64", "x64"),
        ("x86-64", "x64"),
    ]);
    // Ignore case when matching as that's what humans expect.
    
    let arch: String = {
        let arch_lowercase = opt.arch.to_lowercase();

        match arch_aliases.get(arch_lowercase.as_str()) {
            Some(v) => v.to_string(),
            None => arch_lowercase
        }
    };

    // Due to the way Microsoft Visual C++ is configured, we have to resort to the following hack:
    // Call the configuration batch file and then output *all* the environment variables.

    let vcvars: String = {
        let mut args = vec![arch];
        if opt.uwp {
            args.push("uwp".to_string())
        }
        match &opt.sdk {
            Some(v) => args.push(v.to_string()),
            None => {}
        }
        match &opt.toolset {
            Some(v) => args.push(format!("-vcvars_ver={}", v)),
            None => {}
        }
        if opt.spectre {
            args.push("-vcvars_spectre_libs=spectre".to_string());
        }

        // enquote the parameter here
        let vcvarsall = constants.find_vcvarsall(&opt.vsversion)?;
        let vcvarsall_path = match vcvarsall.to_str() {
            Some(v) => v,
            None => bail!("The path to vswhere contains invalid characters: {}", vcvarsall.to_string_lossy())
        };
        let mut v = vec![format!(r#""{}""#, vcvarsall_path)];
        v.extend(args);
        v.join(" ")
    };
    log::debug!("vcvars command-line: {}", vcvars);

    // Unlike the original, which can just shell out and call cmd,
    // here Rust mucks with the escaping of quotes. *flops*
    // See https://internals.rust-lang.org/t/std-process-on-windows-is-escaping-raw-literals-which-causes-problems-with-chaining-commands/8163

    let tmp_batch = {
        let mut batch = Builder::new().suffix(".bat").tempfile()?;
        let arg = format!("set && cls && {} && cls && set", vcvars);
        log::debug!("cmd command: {:?}", arg);
        writeln!(batch, "{}", arg)?;
        batch.flush()?;

        batch
    };

    let mut command = {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(tmp_batch.path());

        cmd
    };
    log::debug!("cmd command: {:?}", command);

    let result = command.output()?;
    let cmd_output_string = result.stdout; 
    log::debug!("vcvars output: \n{}", String::from_utf8_lossy(&cmd_output_string));

    let cmd_error_string = String::from_utf8_lossy(&result.stderr); 
    log::debug!("vcvars error: \n{}", cmd_error_string);

    // form feed
    let cmd_output_parts = cmd_output_string.split(|num| num == &0xC).into_iter().map(|x| String::from_utf8_lossy(x)).collect::<Vec<_>>();
    
    if cmd_output_parts.len() != 3 {
        bail!("Couldn't split the output into pages: {}", cmd_error_string);
    }

    // AFTER this step, you can transform it into strings
    // (otherwise UTF-8 will munge the form feed char)

    let old_environment = cmd_output_parts[0].split('\n');
    let vcvars_output   = cmd_output_parts[1].split('\n');
    let new_environment = cmd_output_parts[2].split('\n');

    // If vsvars.bat is given an incorrect command line, it will print out
    // an error and *still* exit successfully. Parse out errors from output
    // which don't look like environment variables, and fail if appropriate.
    let error_messages = vcvars_output.filter(| i | {
        // Don't print this particular line which will be confusing in output.
        i.contains("[ERROR") && !i.contains("Error in script usage. the correct usage is:")
    }).collect::<Vec<_>>();
    if !error_messages.is_empty() {
        bail!("Invalid parameters\n{}", error_messages.join("\n"));
    }

    // Convert old environment lines into a dictionary for easier lookup.
    let old_env_vars = {
        let mut vars: HashMap<&str, &str> = HashMap::new();
        for i in old_environment {
            // Rust version will take in the shell command line.
            // Skip lines that don't look like environment variables.
            if !i.contains('=') {
                continue;
            }
            match i.split_once('=') {
                Some((name, old_value)) => {
                    vars.insert(name, old_value);
                },
                None => {
                    bail!("Invalid key=value in cmd output: {}", i);
                }
            }
        }
        vars
    };

    // Now look at the new environment and export everything that changed.
    // These are the variables set by vsvars.bat. Also export everything
    // that was not there during the first sweep: those are new variables.
    log::debug!("Environment variables");
    for i in new_environment {
        // vsvars.bat likes to print some fluff at the beginning.
        // Skip lines that don't look like environment variables.
        if !i.contains('=') {
            continue;
        }
        match i.split_once('=') {
            Some((name, new_value)) => {
                let old_value = old_env_vars.get(name);
                // For new variables "old_value === undefined".
                if old_value.is_none() || !matches!(old_value, Some(v) if v.eq_ignore_ascii_case(new_value)) {
                    // Special case for a bunch of PATH-like variables: vcvarsall.bat
                    // just prepends its stuff without checking if its already there.
                    // This makes repeated invocations of this action fail after some
                    // point, when the environment variable overflows. Avoid that.
                    if is_path_variable(name) {
                        let effective_value = filter_path_value(new_value);
                        std::env::set_var(name, effective_value);
                    } else {
                        std::env::set_var(name, new_value);
                    }
                }
            },
            None => {
                bail!("Invalid key=value in cmd output: {}", i);
            }
        }
    }

    log::info!("Configured Developer Command Prompt");

    Ok(())
}

fn main() -> Result<()> {
    env_logger::init();

    let opt = Opt::parse();

    setup_msvcdev_cmd(&opt)?;

    log::info!("Launching: '{}' with args: {:?}", opt.program.to_string_lossy(), opt.args.iter().map(|x| x.to_string_lossy()).collect::<Vec<_>>());

    let cmd = Command::new(opt.program).args(opt.args).spawn().context("Unable to spawn program")?;

    let arc = Arc::new(Mutex::new(cmd));

    #[cfg(unix)] {
        use nix::unistd::Pid;
        use nix::sys::signal::{kill, Signal};

        let arc_handler = arc.clone();
        ctrlc::set_handler(move || {
            let pid = Pid::from_raw(arc_handler.lock().unwrap().id() as i32);
            kill(pid, Signal::SIGINT).context("Unable to kill the program").unwrap();
        }).context("Unable to set the signal handler")?;
    }

    let status = arc.lock().unwrap().wait().context("Unable to wait for the program")?;

    match status.code() {
        Some(i) => process::exit(i),
        None => {
            #[cfg(unix)] {
                use std::os::unix::process::ExitStatusExt;
                process::exit(status.signal().unwrap_or_else(|| 9) + 128);
            }

            #[cfg(windows)]
            process::exit(127);
        }
    };
}
