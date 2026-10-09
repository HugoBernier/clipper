# Changelog

## [0.4.1](https://github.com/HugoBernier/clipper/compare/v0.4.0...v0.4.1) (2026-10-09)


### Bug Fixes

* démarrer sur un PC à deux GPU et signaler un arrêt ([#10](https://github.com/HugoBernier/clipper/issues/10)) ([e04f277](https://github.com/HugoBernier/clipper/commit/e04f2778c3c02e2b9c245ab6c306f7248afe6757))

## [0.4.0](https://github.com/HugoBernier/clipper/compare/v0.3.0...v0.4.0) (2026-10-09)


### Features

* revoir et partager ses clips depuis la fenêtre ([#8](https://github.com/HugoBernier/clipper/issues/8)) ([faa492c](https://github.com/HugoBernier/clipper/commit/faa492c87f51c3addbb63a9e206605455866d85b))

## [0.3.0](https://github.com/HugoBernier/clipper/compare/v0.2.0...v0.3.0) (2026-10-09)


### ⚠ BREAKING CHANGES

* `target_mb` et les préréglages Discord sont retirés de clipper.toml au profit de `quality` et `microphone_volume`. Un ancien fichier est relu sans erreur, mais un préréglage 1080p30 ou 1440p60 donne désormais des clips plus lourds.

### Features

* régler Clipper depuis une fenêtre (qualité, durée, micro, dossier, raccourci) ([#5](https://github.com/HugoBernier/clipper/issues/5)) ([6cfb19b](https://github.com/HugoBernier/clipper/commit/6cfb19bba1be1566edfb621a84b646a95099754c))


### Bug Fixes

* ouvrir les listes au bon endroit et rendre le clavier à la fenêtre ([#7](https://github.com/HugoBernier/clipper/issues/7)) ([eb1fdf8](https://github.com/HugoBernier/clipper/commit/eb1fdf87fe7e17edf7e101f70f86a48df95e9876))

## [0.2.0](https://github.com/HugoBernier/clipper/compare/v0.1.1...v0.2.0) (2026-10-09)


### Features

* **tray:** changer la qualité à chaud depuis le menu de l'icône ([9bd008c](https://github.com/HugoBernier/clipper/commit/9bd008cde4cfbe792ce15ae5dfd249162c949f75))

## [0.1.1](https://github.com/HugoBernier/clipper/compare/v0.1.0...v0.1.1) (2026-10-08)


### Bug Fixes

* **audio:** mixer le micro en mono sur les deux canaux ([5ee2781](https://github.com/HugoBernier/clipper/commit/5ee2781ebe8c4af97e5f04fb591f87e8ca438271))
