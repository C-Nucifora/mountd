#!/usr/bin/env python3
"""Prove seven review regressions reject faulty behavior in a disposable fixture.

This uses the reconciler generator's existing hard-failing SystemOps boundary.
Only the temporary copies are mutated. Cargo must report exactly seven failed
regressions; a passing regression means the control did not demonstrate it.
"""
from pathlib import Path

root = Path(__file__).resolve().parents[3]
generator = root / 'tests/portable-reconciler.py'
source = generator.read_text()
prefix = source[:source.index('    env = os.environ.copy()')]
controls = r"""
    for file in ('Cargo.toml', 'Cargo.lock'):
        p = dest / file
        p.write_text(p.read_text().replace('mountd-reconciler-fixture', 'mountd-negative-fixture'))

    def replace(file, before, after):
        p = dest / 'src' / file
        text = p.read_text()
        assert text.count(before) == 1, (file, before, text.count(before))
        p.write_text(text.replace(before, after))

    # E1: credentials do not retain the engine's operation lease.
    replace('engine.rs', 'KEYCHAIN_TIMEOUT,\n                        lease.clone(),',
            'KEYCHAIN_TIMEOUT,\n                        Arc::new(()),')
    # E3: filesystem preparation happens after the final gates.
    prepare = '        self.ops.prepare_mount(spec.clone(), lease.clone()).await?;\n'
    replace('engine.rs', prepare, '')
    boundary = '        self.ops\n            .mount(spec.clone(), address.to_string(), password, lease.clone())'
    replace('engine.rs', boundary, prepare + boundary)
    # E5: a network snapshot is retained across dependency collection.
    replace('engine.rs', '''        let deps_up = self.ops.check_deps(cfg.clone()).await;
        let snapshot = self.ops.network_snapshot().await;
        let matched_networks = snapshot.matched_networks(&cfg);''', '''        let snapshot = self.ops.network_snapshot().await;
        let matched_networks = snapshot.matched_networks(&cfg);
        let deps_up = self.ops.check_deps(cfg.clone()).await;''')
    # E4: the nested producer only registers when it starts a child/resolver.
    replace('probe.rs', '''        let activity = crate::mountcmd::CommandActivity::begin();
        tasks.spawn(async move {
            let _activity = activity;''', '''        tasks.spawn(async move {''')
    # E2: retain the former proto checks, leaving localhost/netid/family aliases open.
    p = dest / 'src/mountcmd.rs'
    text = p.read_text()
    start = text.index('        if matches!(\n            key.as_str(),\n            "localhost"')
    end = text.index('        if key == "port"', start)
    text = text[:start] + '''        if key == "proto" && !matches!(value, Some("tcp" | "tcp6")) {
            bail!("mount protocol must use TCP");
        }
        if key == "proto" && value == Some("tcp6") && spec.endpoints.iter()
            .any(|e| e.address.parse::<std::net::Ipv6Addr>().is_err()) {
            bail!("proto=tcp6 requires IPv6 literal endpoints");
        }
''' + text[end:]
    p.write_text(text)
    # E6: literal path comparison misses case/normalization collisions.
    replace('mountcmd.rs', 'a.eq_ignore_ascii_case(&b) || !a.is_ascii() || !b.is_ascii()', 'a == b')

    # RF-1: desired-state inspection erases explicitly pending client cleanup.
    replace('launchd.rs', '''    if result
        .as_ref()
        .is_err_and(|error| error.is::<UnsettledMutation>())
    {
        return result;
    }
''', '')

    cases = {
        'launchd.rs': ['pending_or_timed_out_mutations_cannot_be_promoted_by_a_snapshot'],
        'engine.rs': ['cancelled_credentials_retain_claim_and_reload_guard_until_reap',
                      'preparation_is_followed_by_sleep_shutdown_and_probe_gates',
                      'network_identity_is_refreshed_after_dependencies'],
        'probe.rs': ['drain_waits_for_aborted_nested_producer_before_child_registration'],
        'config.rs': ['config_rejects_mount_options_that_escape_the_probe_and_timeout_contract',
                      'rejects_mountpoint_case_and_unicode_aliases'],
    }
    expected = []
    for file, names in cases.items():
        for name in names:
            renamed = 'review_negative_control_' + name
            replace(file, 'fn ' + name + '(', 'fn ' + renamed + '(')
            expected.append(renamed)
    env = os.environ.copy()
    env.setdefault('CARGO_TARGET_DIR', str(source / 'target/portable-daemon'))
    result = subprocess.run(['cargo', 'test', '--locked', '--offline', '--lib',
                             'review_negative_control'], cwd=dest, env=env,
                            text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    print(result.stdout, end='')
    assert result.returncode == 101, result.returncode
    for case in expected:
        assert case + ' ... FAILED' in result.stdout, 'control did not fail: ' + case
    assert '0 passed; 7 failed;' in result.stdout
    print('All seven negative controls failed as expected; production files were unchanged')
"""
exec(compile(prefix + controls, str(generator), 'exec'), {'__file__': str(generator)})
