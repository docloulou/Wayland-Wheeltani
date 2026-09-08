# Wayland-Wheeltani 1.3.3

This patch fixes KDE per-application filtering when Wayland-Wheeltani runs as a systemd user service.

## KDE/systemd fix

`kdotool` writes temporary scripts that KWin must read outside the service sandbox. The service now uses `PrivateTmp=false` and includes `/tmp` in `ReadWritePaths`: sharing `/tmp` makes those scripts visible to KWin, while explicitly allowing writes keeps them writable under `ProtectSystem=strict`.

The fix applies to generated service units and both bundled `contrib` service templates. Other hardening settings and config-path quoting are preserved. There are no changes to scrolling behavior or the configuration format.

Thanks to [@DmitryFX](https://github.com/DmitryFX) for identifying the workaround in [issue #5](https://github.com/docloulou/Wayland-Wheeltani/issues/5). The service fix was merged in [PR #6](https://github.com/docloulou/Wayland-Wheeltani/pull/6).

## Upgrading an existing installation

For the recommended systemd user service, upgrade the binaries, then regenerate the service unit and restart it:

```sh
cargo install wayland-wheeltani --version 1.3.3 --force
wlw --install-service
wlw --restart
```

**Upgrading the binaries alone leaves the old service unit in place**, so existing service installations need the reinstall step to receive this fix.

If you install from a release archive instead, replace both `wayland-wheeltani` and `wlw`, then run the two service commands above.

- **Custom config:** if you originally installed the service with `--config`, add `--config` followed by that same config path to the `wlw --install-service` command above. Keep the path quoted if it contains spaces; do not reinstall with the default path by accident.
- **Manually customized user unit:** save your current unit before reinstalling, then retain or reapply your customizations before restarting. Alternatively, update your existing unit to use `PrivateTmp=false` and add `/tmp` to its existing `ReadWritePaths`, keeping your other settings. Run `systemctl --user daemon-reload` after manual unit edits, then `wlw --restart`.

## Not addressed

The separate mouse-capture behavior reported after `--restart` in [issue #5](https://github.com/docloulou/Wayland-Wheeltani/issues/5) is **not addressed** by this release.
