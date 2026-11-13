#[cfg(test)]
mod round2_credential_path {
    use super::*;
    use crate::config::Config;

    fn config(path: &str, protocol: &str, options: Option<&str>, name: &str) -> String {
        let options = options
            .map(|s| format!("options = {}\n", serde_json::to_string(s).unwrap()))
            .unwrap_or_default();
        let path = serde_json::to_string(path).unwrap();
        format!("[[mount]]\nname = {name:?}\nprotocol = {protocol:?}\nmountpoint = {path}\nshare = \"fake share\"\nusername = \"fake user\"\nexport = \"/fake export\"\n{options}[[mount.endpoint]]\naddress = \"127.0.0.1\"\n")
    }

    #[test]
    fn option_config_and_argv_matrix() {
        for protocol in ["nfs", "smb"] {
            let base =
                Config::from_toml_str(&config("/synthetic/r2", protocol, None, "r2")).unwrap();
            for (token, expected) in [
                ("ro,ro", true),
                (" , , ", true),
                ("soft,,ro,soft", true),
                ("SOFT", false),
                ("SoFt=1", false),
                ("HaRd", false),
                ("bG=0", false),
                ("PrOtO=tcp", false),
                ("noINet6", false),
                ("ro,\tro", true),
                ("ro\tvers=4", false),
                ("port=2049,port=2049", protocol == "nfs"),
                ("port=02049,port=+2049", protocol == "nfs"),
                ("PORT=2049", protocol == "nfs"),
                ("port=2049,port=2050", false),
                ("port=", false),
                ("port=0", false),
                ("port=65536", false),
                ("port=-1", false),
            ] {
                let mut spec = base.mounts[0].clone();
                spec.options = Some(token.into());
                let parsed =
                    Config::from_toml_str(&config("/synthetic/r2", protocol, Some(token), "r2"));
                let argv = mount_argv(&spec, "127.0.0.1", "/synthetic/r2", Some("fake secret"));
                assert_eq!(parsed.is_ok(), expected, "config {protocol} {token:?}");
                assert_eq!(argv.is_ok(), expected, "argv {protocol} {token:?}");
                if let Ok(argv) = argv {
                    let option_index = argv.iter().position(|s| s == "-o").unwrap() + 1;
                    let merged = &argv[option_index];
                    assert!(merged.starts_with(default_options(spec.protocol)));
                    assert_eq!(
                        merged,
                        &validated_options(&parsed.unwrap().mounts[0]).unwrap()
                    );
                    assert_eq!(argv.last().unwrap(), "/synthetic/r2");
                    assert_eq!(argv.len(), if protocol == "nfs" { 5 } else { 6 });
                    assert_eq!(
                        &argv[option_index + 1],
                        if protocol == "nfs" {
                            "127.0.0.1:/fake export"
                        } else {
                            "//fake%20user:fake%20secret@127.0.0.1/fake%20share"
                        }
                    );
                    if token == "ro,ro" {
                        assert_eq!(merged.matches("ro").count(), 1);
                    }
                }
            }
            let mut spec = base.mounts[0].clone();
            spec.options = Some(String::new());
            assert!(validated_options(&spec).is_ok());
            assert!(
                Config::from_toml_str(&config("/synthetic/r2", protocol, Some(""), "r2")).is_err()
            );
        }
        println!("option matrix: 38 config/argv pairs plus two documented empty-string boundaries checked; native parser not executed");
    }

    #[tokio::test]
    async fn local_preparation_errors_and_lexical_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("file");
        std::fs::write(&file, b"sentinel").unwrap();
        let target = root.join("target");
        std::fs::create_dir(&target).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let dangling = root.join("dangling");
        std::os::unix::fs::symlink(root.join("absent"), &dangling).unwrap();
        for path in [
            file.clone(),
            file.join("child"),
            link.clone(),
            link.join("child"),
            dangling.clone(),
            dangling.join("child"),
        ] {
            assert!(ensure_mountpoint(&path, Arc::new(())).await.is_err());
        }
        assert_eq!(std::fs::read(&file).unwrap(), b"sentinel");
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert!(!root.join("absent").exists());
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
        assert_eq!(
            normalize_mountpoint(link.join("../new//leaf/").to_str().unwrap()),
            root.join("new/leaf")
        );
        ensure_mountpoint(&root.join("new/leaf"), Arc::new(()))
            .await
            .unwrap();
        ensure_mountpoint(&root.join("new/leaf"), Arc::new(()))
            .await
            .unwrap();
        assert!(prepare_directory(Path::new("relative")).is_err());
        assert!(prepare_directory(&root.join("new/../elsewhere")).is_err());
        for (a, b, rejected) in [
            ("/synthetic/NAS", "/synthetic/nas//", true),
            ("/synthetic/café", "/synthetic/cafe\u{301}/child", true),
            ("/synthetic/東京", "/synthetic/大阪", true),
            ("/synthetic/a", "/synthetic/ab", false),
            ("/synthetic/a/one", "/synthetic/b/one", false),
        ] {
            assert!(Config::from_toml_str(&config(a, "nfs", None, "a")).is_ok());
            assert!(Config::from_toml_str(&config(b, "nfs", None, "b")).is_ok());
            let text = config(a, "nfs", None, "a") + &config(b, "nfs", None, "b");
            assert_eq!(Config::from_toml_str(&text).is_err(), rejected);
        }
        for path in ["/synthetic/a/../b", "/synthetic/./a"] {
            assert!(Config::from_toml_str(&config(path, "nfs", None, "a")).is_err());
        }
        println!("local paths: six refused walks, unchanged targets, repeat creation, lexical and five alias pairs checked; no mount table or APFS emulation");
    }
}
