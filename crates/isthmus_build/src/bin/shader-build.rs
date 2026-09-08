use std::{env, ffi::OsString, path::Path};

fn main() {
    let [name, source, workspace, output]: [OsString; 4] = env::args_os()
        .skip(1)
        .collect::<Vec<_>>()
        .try_into()
        .expect("usage: shader-build NAME SOURCE WORKSPACE OUTPUT");

    isthmus_build::compile(&name.to_string_lossy(), Path::new(&source), Path::new(&workspace), Path::new(&output))
        .expect("shader build failed");
}
