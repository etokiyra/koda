# AUR packaging for Koda

This directory contains two ready-to-push AUR packages:

| Directory | AUR package | Source |
| --- | --- | --- |
| `koda/` | stable release | GitHub tag `v0.1.3` |
| `koda-git/` | VCS package | `master` from GitHub |

Both install `/usr/bin/koda`, so a normal Arch/Artix `sudo koda` resolves the
binary through the default `secure_path`. Both also install the licence under
`/usr/share/licenses/` and the documentation under `/usr/share/doc/`.

## Validation already performed

On the packaging host (Artix, Arch-compatible):

```bash
makepkg -f --nodeps --noconfirm
namcap PKGBUILD
namcap <built-package>.pkg.tar.zst
```

- `koda 0.1.2-1` built successfully; its package contains `/usr/bin/koda`.
- `koda-git` built successfully; its `pkgver()` derives the version from
  `git describe`, and the package provides and conflicts with `koda`.
- `namcap` reported no findings for either PKGBUILD or package.
- Runtime dependencies are `glibc` and `libgcc`; language runtimes and servers
  are `optdepends`, because Koda itself provisions or discovers them.

## Push the stable package

Replace `~/Projects/koda` with this checkout if it lives elsewhere.

```bash
git clone ssh://aur@aur.archlinux.org/koda.git /tmp/aur-koda
cp ~/Projects/koda/packaging/aur/koda/PKGBUILD /tmp/aur-koda/
cp ~/Projects/koda/packaging/aur/koda/.SRCINFO /tmp/aur-koda/
cd /tmp/aur-koda
git add PKGBUILD .SRCINFO
git commit -m "Initial import: koda 0.1.2"
git push
```

## Push the VCS package

```bash
git clone ssh://aur@aur.archlinux.org/koda-git.git /tmp/aur-koda-git
cp ~/Projects/koda/packaging/aur/koda-git/PKGBUILD /tmp/aur-koda-git/
cp ~/Projects/koda/packaging/aur/koda-git/.SRCINFO /tmp/aur-koda-git/
cd /tmp/aur-koda-git
git add PKGBUILD .SRCINFO
git commit -m "Initial import: koda-git"
git push
```

Only the first push creates the AUR repository. If the names have been taken in
the meantime, rename the `pkgname`/directory consistently before pushing.

## Updating the stable package for a later release

1. Change `pkgver` in `packaging/aur/koda/PKGBUILD`.
2. Refresh the checksum:

   ```bash
   cd packaging/aur/koda
   updpkgsums          # or: makepkg -g >> PKGBUILD after deleting the old sum
   makepkg --printsrcinfo > .SRCINFO
   ```

3. Rebuild and re-run `namcap`, then commit and push the AUR repository.

`koda-git` needs no version edit: `pkgver()` derives the version from
`git describe`. Its `.SRCINFO` is regenerated whenever the PKGBUILD changes.
