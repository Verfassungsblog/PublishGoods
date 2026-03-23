use std::fs;

fn main() {
    println!("Precompiling handlebars frontend templates...");

    // Get list of all files in templates_frontend
    let files = fs::read_dir("templates_frontend").unwrap();
    let mut args = Vec::new();
    for file in files {
        args.push(String::from(file.unwrap().path().to_str().unwrap()));
    }

    let mut args_partials = args.clone();

    // Precompile handlebars partials
    args_partials.push("-p".to_string());
    args_partials.push("-f".to_string());
    args_partials.push("static/js/precompiled_partials.js".to_string());

    let res_partials = std::process::Command::new("handlebars")
        .args(args_partials)
        .output();

    args.push("-f".to_string());
    args.push("static/js/precompiled_templates.js".to_string());

    // Precompile handlebars templates
    let res = std::process::Command::new("handlebars").args(args).output();

    match res {
        Ok(res) => {
            if !res.status.success() {
                panic!(
                    "Failed to precompile handlebars frontend templates: {} {}",
                    String::from_utf8_lossy(&res.stdout),
                    String::from_utf8_lossy(&res.stderr)
                );
            }
        }
        Err(e) => {
            panic!("Failed to precompile handlebars frontend templates: {}", e);
        }
    }

    match res_partials {
        Ok(res) => {
            if !res.status.success() {
                panic!(
                    "Failed to precompile handlebars frontend partials: {} {}",
                    String::from_utf8_lossy(&res.stdout),
                    String::from_utf8_lossy(&res.stderr)
                );
            }
        }
        Err(e) => {
            panic!("Failed to precompile handlebars frontend partials: {}", e);
        }
    }

    println!("Compiling typescript to javascript with tsc...");
    // Compile typescript to javascript with tsc

    let res3 = std::process::Command::new("tsc")
        .args(&[
            "--module",
            "es6",
            "--lib",
            "es2015,dom,dom.Iterable",
            "--target",
            "es6",
            "--outDir",
            "static/js",
            "typescript_old/General.ts",
        ])
        .output()
        .expect("Failed to compile typescript to javascript with tsc");

    let res1 = std::process::Command::new("npm")
        .args(&["--prefix", "typescript", "run", "build"])
        .output()
        .expect("Failed to compile typescript to javascript with npm run build");
    let res2 = std::process::Command::new("tsc")
        .args(&[
            "--module",
            "es6",
            "--lib",
            "es2015,dom,dom.Iterable",
            "--target",
            "es6",
            "--outDir",
            "static/js",
            "typescript_old/Persons.ts",
        ])
        .output()
        .expect("Failed to compile typescript to javascript with tsc");

    println!("cargo:rerun-if-changed=typescript");
    println!("cargo:rerun-if-changed=typescript_old");
    println!("cargo:rerun-if-changed=templates_frontend");
    if !res1.status.success() || !res2.status.success() || !res3.status.success() {
        panic!(
            "Failed to compile typescript to javascript with tsc:\n{}\n{}\n{}",
            String::from_utf8_lossy(&res1.stdout),
            String::from_utf8_lossy(&res2.stdout),
            String::from_utf8_lossy(&res3.stdout)
        );
    }

    // Copy pdf.worker.mjs to static/js to make sure the worker version is always up to date
    fs::copy(
        "typescript/node_modules/pdfjs-dist/build/pdf.worker.mjs",
        "static/js/pdf.worker.mjs",
    )
    .expect("Failed to copy pdf.worker.mjs to static/js");
    println!("cargo:rerun-if-changed=node_modules/pdfjs-dist/build/pdf.worker.mjs");
}
