# claude-jauge

Ou en est ta consommation Claude, sans jamais consommer de quota pour le savoir.

Un widget de barre de menus (macOS, Linux, Windows) et un CLI headless, qui
affichent le pourcentage restant et le temps avant la remise a zero des trois
fenetres de quota : **session 5h**, **semaine 7j** et **overage** mensuel.

![les etats de la jauge](docs/jauge.png)

## D'ou viennent les chiffres

L'API renvoie l'etat des quotas dans les en-tetes de **chacune** de ses reponses :

```
anthropic-ratelimit-unified-5h-utilization: 0.73     <- 73 % de la fenetre 5h
anthropic-ratelimit-unified-5h-reset: 1784225400     <- epoch de la remise a zero
anthropic-ratelimit-unified-5h-status: allowed       <- allowed | allowed_warning | ...
anthropic-ratelimit-unified-7d-utilization: 0.14
anthropic-ratelimit-unified-overage-utilization: 0.0
anthropic-ratelimit-unified-fallback: available
```

Une requete minimale suffit donc a tout apprendre : on demande un jeton au plus
petit modele, et on jette la reponse. Seuls les en-tetes comptent. Le jeton OAuth
est celui que Claude Code a deja pose sur la machine.

> **Ces en-tetes ne sont pas documentes publiquement.** Ils peuvent changer sans
> preavis. Tout est donc optionnel dans le code : un champ absent s'affiche `?`,
> il ne fait rien planter.

## Ce qui le distingue d'un simple poll

L'idee vient de [Clawdmeter](https://github.com/HermannBjorgvin/Clawdmeter), le
petit ecran ESP32 de Hermann Bjorgvin. Son daemon interroge l'API toutes les
60 secondes, ce qui a un effet de bord genant : **une sonde est une requete comme
une autre, donc elle OUVRE une fenetre de 5 heures s'il n'y en a pas deja une en
cours**. Le widget demarre alors le compteur a ta place, et tu retrouves une
fenetre deja entamee quand tu te mets enfin au travail.

`claude-jauge` s'appuie sur deux constats :

1. les compteurs ne bougent que si Claude Code parle a l'API, ce qui laisse une
   trace datee dans `~/.claude/projects` ;
2. le compte a rebours et la retombee a zero d'une fenetre echue se calculent
   parfaitement en local, sans reseau.

D'ou la regle, dans [`meter::should_probe`](crates/core/src/meter.rs) :

| Situation | Sonde ? |
| --- | --- |
| Claude Code a ecrit depuis le dernier releve | oui |
| Rien n'a bouge depuis le dernier releve | non, le cache suffit |
| Moins de 60 s depuis la derniere sonde | non, plancher anti-rafale |
| Premier lancement, activite il y a moins de 5 h | oui, une fenetre est deja ouverte |
| Premier lancement, plus rien depuis 5 h | **non**, ca ouvrirait une fenetre pour rien |
| Clic sur « Rafraichir » | oui, c'est un geste volontaire |

Autrement dit : au repos, zero requete. Le compte a rebours continue pourtant
d'avancer a l'ecran.

## Architecture

```
claude-jauge/
├── crates/core/    lib pure : jeton, sonde, en-tetes, cache, politique de releve,
│                   et meme le dessin de l'icone. Aucune dependance graphique.
├── crates/cli/     binaire `claude-jauge` : headless, pour un serveur ou tmux.
└── apps/tray/      Tauri 2 + Svelte 5 : le widget de barre de menus.
```

L'app Tauri est **hors du workspace Cargo** (`exclude` dans le `Cargo.toml`
racine) : elle se lie a webkit2gtk, donc l'inclure ferait echouer `cargo test`
sur toute machine sans serveur graphique. Le coeur et le CLI, eux, se compilent
et se testent partout.

## Le CLI

```sh
cargo build --release -p claude-jauge-cli
./target/release/claude-jauge
```

```
  Claude · pro

  Session 5h  ▓▓▓▓▓▓▓▓▓▓   96%  reste   4%   reset 20:10 (45m)  limite proche
  Semaine 7j  ▓▓░░░░░░░░   16%  reste  84%   reset dim 19/07 09:00 (2j13h)
  Overage     ░░░░░░░░░░    0%  reste 100%   reset sam 01/08 02:00 (15j06h)

  releve a l'instant · repli dispo a 50%
```

| Option | Effet |
| --- | --- |
| `-j, --json` | Sortie JSON, pour une barre de statut ou un script. |
| `-w, --watch [SECS]` | Rafraichit en continu (5 s par defaut). |
| `-f, --force` | Sonde meme si rien n'a bouge. A froid, **ouvre une fenetre de 5h**. |
| `--no-color` | Aucune couleur (`NO_COLOR` est respecte aussi). |

## Le widget

```sh
cd apps/tray
npm install
npm run tauri dev      # developpement
npm run tauri build    # paquet distribuable
```

* Clic gauche sur l'icone : la popover s'ouvre.
* Clic droit : « Rafraichir maintenant » et « Quitter ».
* Le titre a cote de l'icone (`96% · 45m`) s'affiche tel quel dans la barre macOS.
* `Echap` referme, `Cmd/Ctrl+R` sonde.

L'icone est un anneau dessine en RGBA a la main, sans bibliotheque de rendu, dans
[`core/src/icon.rs`](crates/core/src/icon.rs). Elle vit dans le coeur, et non
dans l'app, pour rester testable sans serveur graphique.

### Prerequis de build

* **Tous** : [Rust](https://rustup.rs) et Node 18+.
* **macOS** : Xcode Command Line Tools.
* **Linux** : `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`,
  `librsvg2-dev`, `libxdo-dev`, `build-essential`, `pkg-config`.
  Sous GNOME/Wayland, le tray reclame en plus une extension AppIndicator.
* **Windows** : WebView2 (deja present sur Windows 11).

Les icones de l'application se generent une fois avec `npm run tauri icon`.

## Les trois etats d'une fenetre

Le statut n'est pas binaire. A 95 % d'une fenetre, l'API renvoie
`allowed_warning` : les requetes passent toujours, c'est un avertissement. Le
code traite donc tout prefixe `allowed` comme « ca passe », pour qu'une variante
future n'apparaisse pas a tort comme un refus.

| `status` | Affichage |
| --- | --- |
| `allowed` | rien |
| `allowed_warning` | « limite proche », en ambre |
| autre chose | « epuise », en rouge |

## Vie privee

Le jeton OAuth est lu la ou Claude Code l'a mis (trousseau macOS, ou
`~/.claude/.credentials.json`), et n'est envoye qu'a `api.anthropic.com`. Il
n'est jamais reecrit : les jetons de rafraichissement tournent a chaque usage,
donc en consommer un ici risquerait d'invalider la session de Claude Code
elle-meme. Quand le jeton expire, `claude-jauge` le dit et laisse Claude Code le
renouveler.

Le seul fichier ecrit est le cache du dernier releve
(`~/.cache/claude-jauge/state.json` sous Linux).

## Tests

```sh
cargo test        # coeur + CLI, sans serveur graphique
```

45 tests couvrent le parsing des en-tetes (a partir d'un releve reel), la
politique de sonde, le cache, la detection d'activite et le dessin de l'icone.

## Licence

MIT.
