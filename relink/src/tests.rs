use super::*;

fn scratch(name: &str) -> PathBuf {
    let path = env::temp_dir().join(format!("eon-relink-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn patchelf(arguments: &[&str], file: &Path) {
    let status = Command::new("patchelf").args(arguments).arg(file).status();
    assert!(status.unwrap().success());
}

#[test]
fn closure_ships_what_the_root_reaches_without_the_exclusions() {
    let graph: &[(&str, &[&str])] = &[
        (
            "/nix/store/a-eon",
            &[
                "/nix/store/b-lib",
                "/nix/store/c-glibc",
                "/nix/store/d-mesa",
            ],
        ),
        (
            "/nix/store/b-lib",
            &["/nix/store/b-lib", "/nix/store/c-glibc"],
        ),
        ("/nix/store/c-glibc", &["/nix/store/e-idn"]),
        (
            "/nix/store/d-mesa",
            &["/nix/store/b-lib", "/nix/store/f-llvm"],
        ),
        ("/nix/store/e-idn", &[]),
        ("/nix/store/f-llvm", &[]),
    ];
    let registration: String = graph
        .iter()
        .map(|(path, references)| {
            let listed: String = references.iter().map(|path| format!("\n{path}")).collect();
            format!("{path}\nsha256:0\n1\n\n{}{listed}\n", references.len())
        })
        .collect();
    let exclude = ["/nix/store/c-glibc".into(), "/nix/store/d-mesa".into()];
    let (kept, pruned) = closure(&registration, "/nix/store/a-eon", &exclude).unwrap();
    assert_eq!(kept, ["/nix/store/a-eon", "/nix/store/b-lib"]);
    assert_eq!(pruned, 2);

    let stale = ["/nix/store/z-glibc".into()];
    let error = closure(&registration, "/nix/store/a-eon", &stale).unwrap_err();
    assert!(error.to_string().contains("z-glibc"));
}

#[test]
fn gates_reject_seeded_violations() {
    let root = scratch("gates");
    let package = root.join("nix/store/a-demo");
    fs::create_dir_all(package.join("bin")).unwrap();
    fs::create_dir_all(package.join("lib")).unwrap();
    let demo = package.join("bin/demo");
    fs::copy(env::current_exe().unwrap(), &demo).unwrap();
    let link = package.join("bin/link");
    symlink("../lib/libeon-missing.so.1", &link).unwrap();
    let host = [
        "libc.so.6",
        "libm.so.6",
        "libgcc_s.so.1",
        "ld-linux-x86-64.so.2",
    ];
    let gates = Gates {
        floor: vec![2, 42],
        host: host.into_iter().map(Into::into).collect(),
    };
    let kinds = |elves: &[&PathBuf]| -> Vec<&str> {
        let elves: Vec<_> = elves.iter().map(|&elf| elf.clone()).collect();
        let report = check(&root, &elves, std::slice::from_ref(&link), &gates).unwrap();
        report
            .violations
            .into_iter()
            .map(|(kind, _)| kind)
            .collect()
    };

    // Outside $ORIGIN, not a whole $ORIGIN component, out of the tree, the working directory.
    let runpath = format!("/opt/lib:$ORIGINx:$ORIGIN/{}:", "../".repeat(9));
    patchelf(
        &["--set-interpreter", "/opt/ld.so", "--set-rpath", &runpath],
        &demo,
    );
    patchelf(&["--add-needed", "libeon-missing.so.1"], &demo);
    assert_eq!(
        kinds(&[&demo]),
        [
            "symlink",
            "interpreter",
            "runpath",
            "runpath",
            "runpath",
            "runpath",
            "needed"
        ]
    );

    patchelf(
        &[
            "--set-interpreter",
            INTERPRETER,
            "--set-rpath",
            "$ORIGIN/../lib",
        ],
        &demo,
    );
    assert_eq!(kinds(&[&demo]), ["symlink", "needed"]);

    // Neither an empty file nor an executable provides a library; a shared object does,
    // such as the libgcc_s this test runs with.
    let provider = package.join("lib/libeon-missing.so.1");
    fs::write(&provider, "").unwrap();
    assert_eq!(kinds(&[&demo]), ["needed"]);
    fs::copy(&demo, &provider).unwrap();
    // The copy needs itself as well.
    assert_eq!(kinds(&[&demo, &provider]), ["needed", "needed"]);
    let maps = fs::read_to_string("/proc/self/maps").unwrap();
    let libgcc = maps
        .lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .find(|path| path.ends_with("/libgcc_s.so.1"))
        .unwrap();
    fs::copy(libgcc, &provider).unwrap();
    fs::set_permissions(&provider, fs::Permissions::from_mode(0o644)).unwrap();
    patchelf(&["--remove-rpath"], &provider);
    assert!(kinds(&[&demo, &provider]).is_empty());

    // The loader stops at the first candidate, so an unusable one hides the provider.
    fs::create_dir(package.join("early")).unwrap();
    fs::write(package.join("early/libeon-missing.so.1"), "").unwrap();
    patchelf(&["--set-rpath", "$ORIGIN/../early:$ORIGIN/../lib"], &demo);
    assert_eq!(kinds(&[&demo, &provider]), ["needed"]);
    patchelf(&["--set-rpath", "$ORIGIN/../lib"], &demo);

    // A name with a slash is opened from the working directory, not through RUNPATH.
    let slashed = package.join("bin/slashed");
    fs::copy(&demo, &slashed).unwrap();
    patchelf(&["--add-needed", "../lib/libeon-missing.so.1"], &slashed);
    assert_eq!(kinds(&[&slashed, &provider]), ["needed"]);

    // An object built against a newer glibc: rename the GLIBC_2.34 need every glibc 2.34+
    // program has. patchelf leaves the old string table in place, so rename each copy.
    let mut bytes = fs::read(&demo).unwrap();
    assert!(rename(&mut bytes, b"GLIBC_2.34\0", b"GLIBC_2.43\0"));
    let newer = package.join("bin/newer");
    fs::write(&newer, bytes).unwrap();
    assert_eq!(kinds(&[&newer, &provider]), ["glibc-floor"]);

    // Version needs that readelf cannot list fail rather than pass.
    let stripped = package.join("bin/stripped");
    let status = Command::new("objcopy")
        .arg("--strip-section-headers")
        .args([&demo, &stripped])
        .status();
    assert!(status.unwrap().success());
    assert_eq!(kinds(&[&stripped, &provider]), ["glibc-floor"]);

    // The loader follows bin/jump before "..", so this searches other/lib, not lib.
    fs::create_dir_all(package.join("other/deep/deeper")).unwrap();
    symlink("../other/deep/deeper", package.join("bin/jump")).unwrap();
    patchelf(&["--set-rpath", "$ORIGIN/jump/../../lib"], &demo);
    assert_eq!(kinds(&[&demo, &provider]), ["needed"]);
    // Nor does a path step through a file: demo/.. is no directory.
    patchelf(&["--set-rpath", "$ORIGIN/demo/../../lib"], &demo);
    assert_eq!(kinds(&[&demo, &provider]), ["needed"]);

    // A library found through lib/ takes lib/ as its $ORIGIN, so one that lies elsewhere
    // may do so only without a RUNPATH of its own.
    patchelf(&["--set-rpath", "$ORIGIN/../lib"], &demo);
    let elsewhere = package.join("other/libeon-missing.so.1");
    fs::rename(&provider, &elsewhere).unwrap();
    symlink("../other/libeon-missing.so.1", &provider).unwrap();
    assert!(kinds(&[&demo, &elsewhere]).is_empty());
    patchelf(&["--set-rpath", "$ORIGIN"], &elsewhere);
    assert_eq!(kinds(&[&demo, &elsewhere]), ["needed"]);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn substitutes_ship_under_the_original_name() {
    let root = scratch("substitute");
    let replacement = root.join("r-lib");
    fs::create_dir_all(replacement.join("share")).unwrap();
    let data = "/nix/store/r-lib/share:/nix/store/r-lib/terminfo";
    fs::write(replacement.join("share/data"), data).unwrap();
    symlink("/nix/store/r-lib/share/data", replacement.join("link")).unwrap();
    fs::create_dir(root.join("store")).unwrap();
    let mut relink = Relink {
        kept: HashSet::from(["o-lib"]),
        rename: Some(("r-lib", "o-lib")),
        ..Relink::default()
    };
    let original = root.join("store/o-lib");
    relink.copy(&replacement, &original, 0).unwrap();
    assert_eq!(
        fs::read_to_string(original.join("share/data")).unwrap(),
        "/nix/store/o-lib/share:/nix/store/o-lib/terminfo"
    );
    assert_eq!(
        fs::read_link(original.join("link")).unwrap(),
        Path::new("../o-lib/share/data")
    );
    fs::remove_dir_all(root).unwrap();
}
