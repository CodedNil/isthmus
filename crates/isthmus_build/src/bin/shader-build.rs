use isthmus_build::ShaderBuild;
use std::{env, path::PathBuf};

fn main() {
    let mut args = env::args_os().skip(1);
    let name = required(&mut args, "name");
    let source = required(&mut args, "source");
    let isthmus = required(&mut args, "isthmus");
    let workspace = required(&mut args, "workspace");
    let output = required(&mut args, "output");

    if args.next().is_some() {
        panic!("unexpected argument");
    }

    ShaderBuild {
        name: name.to_string_lossy().into_owned(),
        source: PathBuf::from(source),
        isthmus: PathBuf::from(isthmus),
        workspace: PathBuf::from(workspace),
        output: PathBuf::from(output),
    }
    .build()
    .expect("shader build failed");
}

fn required(args: &mut impl Iterator<Item = std::ffi::OsString>, name: &str) -> std::ffi::OsString {
    args.next().unwrap_or_else(|| panic!("missing {name} argument"))
}
