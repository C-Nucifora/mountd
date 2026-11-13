use crate::{config::Config, mountcmd, mtab};

const HOSTS: [&str; 8] = [
    "192.0.2.17",
    "nas.example",
    "NAS.Example.",
    "2001:db8::1",
    "2001:0DB8:0000:0000:0000:0000:0000:0001",
    "2001:db8::",
    "::",
    "::ffff:192.0.2.17",
];
const MP: &str = "/synthetic/round4/share";

fn quote(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}

fn body(
    protocol: &str,
    hosts: &[&str],
    username: Option<&str>,
    remote: &str,
    path: &str,
) -> String {
    let field = if protocol == "smb" { "share" } else { "export" };
    let mut text = format!(
        "[[mount]]\nname = 'r4'\nprotocol = '{}'\n{} = {}\nmountpoint = {}\n",
        protocol,
        field,
        quote(remote),
        quote(path)
    );
    if let Some(user) = username {
        text += &format!("username = {}\n", quote(user));
    }
    for host in hosts {
        text += &format!("[[mount.endpoint]]\naddress = {}\n", quote(host));
    }
    text
}

fn entry(path: &str, source: &str) -> mtab::MountEntry {
    mtab::MountEntry {
        fstype: "smbfs".into(),
        from: source.into(),
        on: path.into(),
    }
}

#[test]
fn smb_config_to_argv_to_address_preserves_identity() {
    let users = [
        None,
        Some("user"),
        Some("u@other"),
        Some("a/b:c"),
        Some("DOMAIN;u"),
        Some("é%20"),
    ];
    let shares = [
        "share",
        "media library",
        "a/b",
        "s@host:x",
        "%2F%40",
        "共享",
    ];
    let passwords = [None, Some("synthetic"), Some("p@:/%[]é")];
    let mut count = 0;
    for host in HOSTS {
        for user in users {
            for share in shares {
                let cfg = Config::from_toml_str(&body("smb", &[host], user, share, MP)).unwrap();
                let spec = &cfg.mounts[0];
                assert_eq!(spec.endpoints[0].address, host);
                for password in passwords {
                    let argv = mountcmd::mount_argv(spec, host, MP, password).unwrap();
                    let source = &argv[argv.len() - 2];
                    assert_eq!(source, &mountcmd::smb_source(user, password, host, share));
                    assert_eq!(
                        mtab::source_address(source).as_deref(),
                        Some(host),
                        "{source}"
                    );
                    assert_eq!(
                        source.matches('/').count(),
                        3,
                        "component escaped out of URL"
                    );
                    assert_eq!(source.matches('@').count(), usize::from(user.is_some()));
                    assert_eq!(argv.last().unwrap(), MP);
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 864);
    // Independent exact bytes ensure the composition cannot hide a shared encoding mistake.
    assert_eq!(
        mountcmd::smb_source(Some("u@/"), Some("p:%"), "2001:db8::", "a/bé"),
        "//u%40%2F:p%3A%25@[2001:db8::]/a%2Fb%C3%A9"
    );
    println!("SMB: {count} config/argv/parser rows plus exact encoded-byte oracle");
}

#[test]
fn nfs_config_to_argv_to_address_preserves_identity() {
    let exports = [
        "/",
        "/export",
        "/export/",
        "//export//leaf",
        "/space name",
        "/a@b:c",
        "/%2F",
        "/共享",
    ];
    let mut count = 0;
    for host in HOSTS {
        for export in exports {
            let cfg = Config::from_toml_str(&body("nfs", &[host], None, export, MP)).unwrap();
            let argv = mountcmd::mount_argv(&cfg.mounts[0], host, MP, None).unwrap();
            let authority = if host.contains(':') {
                format!("[{host}]")
            } else {
                host.to_owned()
            };
            let source = &argv[argv.len() - 2];
            assert_eq!(source, &format!("{authority}:{export}"));
            assert_eq!(
                mtab::source_address(source).as_deref(),
                Some(host),
                "{source}"
            );
            assert_eq!(argv.last().unwrap(), MP);
            count += 1;
        }
    }
    assert_eq!(count, 64);
    println!("NFS: {count} config/argv/parser rows, exact export bytes retained");
}

#[test]
fn authority_syntax_is_rejected_at_configuration_boundary() {
    let invalid = [
        "",
        " host",
        "host ",
        "a\tb",
        "a\nb",
        "a\0b",
        "host/share",
        "host\\share",
        "u@host",
        "host?x",
        "host#x",
        "host%20",
        "[2001:db8::1]",
        "[2001:db8::1]:445",
        "host:445",
        "192.0.2.17:2049",
        "smb://host/share",
        "//host/share",
        "host:/export",
        "fe80::1%en0",
        "2001:db8:::1",
        "[host]",
        "-host",
        "-o",
    ];
    let mut count = 0;
    for protocol in ["smb", "nfs"] {
        for host in invalid {
            assert!(
                Config::from_toml_str(&body(protocol, &[host], None, "/share", MP)).is_err(),
                "{protocol}: {host:?}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 48);
    // Existing documented negative examples, not a new native source grammar.
    for source in ["", "/dev/disk1s1", "/dev/disk1s1/", "/dev/disk@host:/x"] {
        assert_eq!(mtab::source_address(source), None, "{source}");
    }
    println!("48 authority rejections and four local/empty source negatives");
}

#[test]
fn endpoint_order_and_mount_names_preserve_exact_values() {
    let hosts = ["NAS.Example.", "192.0.2.17", "2001:0DB8::1"];
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut count = 0;
    for protocol in ["smb", "nfs"] {
        for order in orders {
            let ordered = order.map(|i| hosts[i]);
            let cfg = Config::from_toml_str(&body(protocol, &ordered, Some("u@x"), "/share", MP))
                .unwrap();
            let spec = &cfg.mounts[0];
            assert_eq!(
                spec.endpoints
                    .iter()
                    .map(|e| e.address.as_str())
                    .collect::<Vec<_>>(),
                ordered
            );
            for (rank, host) in ordered.iter().enumerate() {
                let argv = mountcmd::mount_argv(spec, host, MP, None).unwrap();
                let address = mtab::source_address(&argv[argv.len() - 2]).unwrap();
                // Same pure equality expression used by engine::migration_target.
                assert_eq!(
                    spec.endpoints.iter().position(|e| e.address == address),
                    Some(rank)
                );
                count += 1;
            }
        }
    }
    let names = ["nas", "NAS", " nas ", "café", "cafe\u{301}"];
    let text: String = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            body(
                "smb",
                &["nas.example"],
                None,
                "share",
                &format!("/synthetic/round4/m{i}"),
            )
            .replace("name = 'r4'", &format!("name = {}", quote(name)))
        })
        .collect();
    let cfg = Config::from_toml_str(&text).unwrap();
    for (i, name) in names.iter().enumerate() {
        assert_eq!(
            cfg.mount(name).unwrap().mountpoint,
            format!("/synthetic/round4/m{i}")
        );
        assert_eq!(cfg.mounts[i].name, *name);
    }
    assert_eq!(count, 36);
    println!(
        "36 rank compositions across six permutations and two protocols; five exact mount names"
    );
}

#[test]
fn normalized_mountpoints_match_only_the_exact_entry() {
    let paths = [
        "/",
        "/synthetic/round4/a",
        "/synthetic/round4/ab",
        "/synthetic/round4/a/child",
        "/synthetic/round4/A",
        "/synthetic/round4/café",
        "/synthetic/round4/cafe\u{301}",
        "/synthetic/round4/a ",
    ];
    let entries: Vec<_> = paths
        .iter()
        .enumerate()
        .map(|(i, path)| entry(path, &format!("//host{i}/share")))
        .collect();
    let mut count = 0;
    for (i, path) in paths.iter().enumerate() {
        for suffix in ["", "/", "///"] {
            let normalized = mountcmd::normalize_mountpoint(&format!("{path}{suffix}"));
            let matched = mtab::find_by_mountpoint(&entries, normalized.to_str().unwrap()).unwrap();
            assert_eq!(matched.from, format!("//host{i}/share"));
            for (j, candidate) in entries.iter().enumerate() {
                assert_eq!(
                    mtab::find_by_mountpoint(
                        std::slice::from_ref(candidate),
                        normalized.to_str().unwrap()
                    )
                    .is_some(),
                    i == j
                );
                count += 1;
            }
        }
    }
    assert_eq!(count, 192);
    for input in [
        "/synthetic//round4///share/",
        "/synthetic/round4/share",
        "share",
    ] {
        let text = "[defaults]\nmount_root = '/synthetic//round4/'\n".to_owned()
            + &body("smb", &["nas.example"], None, "share", input);
        let cfg = Config::from_toml_str(&text).unwrap();
        let normalized = mountcmd::normalize_mountpoint(&cfg.mounts[0].mountpoint);
        assert_eq!(normalized.to_str(), Some(MP));
        let argv = mountcmd::mount_argv(
            &cfg.mounts[0],
            "nas.example",
            normalized.to_str().unwrap(),
            None,
        )
        .unwrap();
        let table = [entry(MP, &argv[argv.len() - 2])];
        assert!(mtab::find_by_mountpoint(&table, argv.last().unwrap()).is_some());
    }
    for path in [
        "",
        "/",
        "///",
        "/synthetic/./share",
        "/synthetic/../share",
        "~",
        "~/",
    ] {
        assert!(
            Config::from_toml_str(&body("smb", &["nas.example"], None, "share", path)).is_err(),
            "{path}"
        );
    }
    println!("192 exact path comparisons; three config/normalizer/argv/matcher compositions; seven invalid paths");
}
