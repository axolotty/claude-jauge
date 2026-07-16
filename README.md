# claude-jauge

Ou en est ta consommation Claude, sans jamais consommer de quota pour le savoir.

Un widget de barre de menus (macOS, Linux, Windows) et un CLI headless, qui
affichent le pourcentage restant et le temps avant la remise a zero des trois
fenetres de quota : **session 5h**, **semaine 7j** et **overage** mensuel.

Les chiffres valent pour **tout Claude**, pas seulement Claude Code : sur Pro et
Max, claude.ai, l'app de bureau et Claude Code puisent dans le meme pool. C'est
le sens du mot `unified` dans le nom des en-tetes.

![les etats de la jauge](docs/jauge.png)

## Deux facons d'obtenir les chiffres

### 1. La statusline, la voie normale

Claude Code passe l'etat des quotas a sa statusline, a chaque rendu de sa barre.
Une ligne dans `~/.claude/settings.json` suffit :

```json
{ "statusLine": { "type": "command", "command": "claude-jauge statusline" } }
```

Il recoit alors, sur son entree standard :

```json
{
  "model": { "display_name": "Opus" },
  "context_window": { "used_percentage": 8 },
  "rate_limits": {
    "five_hour": { "used_percentage": 23.5, "resets_at": 1738425600 },
    "seven_day": { "used_percentage": 41.2, "resets_at": 1738857600 }
  }
}
```

`claude-jauge statusline` affiche ces chiffres dans la barre, et en depose une
copie dans le cache que lit le widget. Tant que le cache est ainsi alimente,
**aucune sonde n'est envoyee** : ni appel API, ni jeton, ni en-tete non publie.
La documentation precise d'ailleurs que la statusline ne consomme aucun token.

C'est le chemin a privilegier, et de loin. Ses limites : `rate_limits` n'existe
que pour les abonnes Pro et Max, et seulement apres la premiere reponse de l'API
dans la session ; il n'y a ni overage ni fallback ; et elle ne s'execute que
pendant que Claude Code tourne. Entre deux rendus, le compte a rebours avance en
local, et les compteurs ne bougent de toute facon que si tu consommes.

> Attention a l'echelle si tu lis le code : ici `used_percentage` vaut deja 0 a
> 100 (`23.5` pour 23,5 %), alors que les en-tetes HTTP du repli expriment une
> fraction (`0.73` pour 73 %).

### 2. La sonde, en repli

Pour les machines ou Claude Code ne tourne pas. L'API renvoie l'etat des quotas
dans les en-tetes de **chacune** de ses reponses :

```
anthropic-ratelimit-unified-5h-utilization: 0.73     <- 73 % de la fenetre 5h
anthropic-ratelimit-unified-5h-reset: 1784225400     <- epoch de la remise a zero
anthropic-ratelimit-unified-5h-status: allowed       <- allowed | allowed_warning | ...
anthropic-ratelimit-unified-7d-utilization: 0.14
anthropic-ratelimit-unified-overage-utilization: 0.0
anthropic-ratelimit-unified-fallback: available
```

Une requete minimale suffit donc a tout apprendre : on demande un jeton au plus
petit modele, et on jette la reponse. Seuls les en-tetes comptent.

Il faut un jeton **OAuth d'abonnement**, celui que Claude Code pose sur la
machine. Une cle API de `console.anthropic.com` ne conviendrait pas : elle ouvre
sur le pay-as-you-go, dont les quotas sont d'une autre nature (requetes/min,
tokens/min) et qui ne renvoie aucun en-tete `unified-*`.

## Surveiller depuis une machine sans Claude Code

Le cas classique : coder sur un serveur, surveiller depuis son portable. Les
quotas etant globaux au compte, peu importe d'ou on regarde.

La bonne facon est de rapatrier le **releve**, pas le jeton :

```json
{ "remote": { "command": "ssh serveur cat .cache/claude-jauge/state.json" } }
```

Le serveur tient son cache a jour tout seul, par sa statusline. Le poste qui
regarde ne fait que le lire : aucun jeton ne circule, rien n'expire, aucune
requete n'est envoyee a l'API, et le cache local sert de tampon (une lecture par
minute au plus). Quand la machine est injoignable, le dernier releve connu reste
affiche, avec son age reel et la raison de l'echec.

L'autre facon, rapatrier le jeton via `credentials.command`, marche aussi mais
elle est fragile : **rien ne rafraichit un jeton OAuth a part Claude Code**. Des
qu'il s'arrete sur la machine d'origine, le fichier perime en quelques heures et
le widget tombe en panne. A reserver aux cas ou l'autre machine n'a pas
`claude-jauge`.

## Ou trouver le jeton (repli uniquement)

Rien de tout ceci n'est necessaire avec la statusline ni avec `remote` : ni
l'une ni l'autre n'utilise de jeton. Ce qui suit ne vaut que pour la sonde.

Par defaut, la ou Claude Code le range : trousseau macOS
(`Claude Code-credentials`), ou `~/.claude/.credentials.json`.

Mais **Claude Code n'a pas besoin de tourner sur la machine qui porte le
widget**. Les quotas sont globaux au compte, donc surveiller depuis un poste ou
l'on ne code pas est un usage parfaitement legitime, et c'est meme le cas le plus
courant : coder sur un serveur, surveiller depuis son portable. Le bloc
`credentials` de la config sert a ca.

```json
{
  "credentials": {
    "command": "ssh serveur cat .claude/.credentials.json"
  }
}
```

| Champ | Quand | Remarque |
| --- | --- | --- |
| `command` | Claude Code vit ailleurs | **Le plus solide.** Rejoue a chaque sonde, donc le jeton reste frais. Le jeton ne touche jamais le disque local. |
| `path` | Fichier a un endroit inhabituel | Un simple chemin. |
| `token` | Depannage | Un jeton OAuth expire en quelques heures : il sera mort avant ce soir. |

Les trois sont essayes dans cet ordre, et la detection automatique ne joue que si
aucun n'est renseigne. Une source renseignee qui echoue est une erreur franche :
retomber en silence sur la detection automatique masquerait une faute de frappe.

Toute commande est coupee au bout de 10 s. Sans cette limite, une autorisation de
trousseau restee sans reponse, ou un `ssh` vers une machine eteinte, figerait le
widget sans un mot.

> **Ces en-tetes ne sont pas documentes publiquement.** Ils peuvent changer sans
> preavis, et c'est une raison de plus de preferer la statusline. Tout est donc
> optionnel dans le code : un champ absent est tu, il ne fait rien planter.

## Quand sonder : un arbitrage sans bonne reponse

*Cette section ne concerne que le repli. Avec la statusline configuree, rien de
tout ceci ne se declenche.*

L'idee vient de [Clawdmeter](https://github.com/HermannBjorgvin/Clawdmeter), le
petit ecran ESP32 de Hermann Bjorgvin. Son daemon interroge l'API toutes les
60 secondes, ce qui a un effet de bord genant : **une sonde est une requete comme
une autre, donc elle ouvre une fenetre de 5 heures s'il n'y en a pas deja une en
cours**. Le widget demarre alors le compteur a ta place.

L'inverse ne marche pas non plus. On pourrait ne sonder que quand Claude Code a
ecrit dans `~/.claude/projects`, mais les quotas sont partages avec claude.ai :
**une conversation dans le navigateur ne laisse aucune trace locale**. Se fier
aux transcripts, c'est afficher des chiffres faux des que la consommation vient
d'ailleurs, et n'afficher rien du tout sur une machine ou Claude Code ne tourne
pas.

Les deux exigences sont incompatibles, donc c'est un reglage
([`Policy`](crates/core/src/config.rs)) :

| | `fidele` (defaut) | `prudent` |
| --- | --- | --- |
| Cadence | toutes les 2 min, quoi qu'il arrive | seulement quand c'est gratuit |
| Suit claude.ai | oui | pendant une fenetre ouverte |
| Ouvre une fenetre au repos | **oui** | jamais |
| Apres un reset, au repos | juste | affiche 0 % jusqu'au prochain geste |

Sonder est « gratuit » quand une fenetre court deja, ou quand Claude Code a ecrit
depuis le dernier releve : dans les deux cas la fenetre est ouverte de toute
facon. Un plancher de 60 s s'applique aux deux politiques, et « Rafraichir »
sonde toujours.

Entre deux sondes, rien n'est demande au reseau : le compte a rebours et la
retombee a zero d'une fenetre echue se calculent en local.

> **Une reserve honnete.** Que la sonde ouvre reellement une fenetre repose sur
> la documentation d'Anthropic (« la fenetre demarre a ton premier message »), et
> n'a pas ete verifie experimentalement ici : il faudrait observer un compte
> reste inactif plus de 5 heures. La politique `fidele` est le bon choix dans les
> deux cas ; `prudent` n'est utile que si l'hypothese est vraie.

### Le reglage

Le widget expose « Sonde fidele » et « Sonde prudente » dans son menu (clic
droit). Le CLI accepte `--policy fidele|prudent` pour un seul appel.

Le fichier, ecrit par le widget :

```json
{ "policy": "fidele" }
```

| Plateforme | Chemin |
| --- | --- |
| Linux | `~/.config/claude-jauge/config.json` |
| macOS | `~/Library/Application Support/claude-jauge/config.json` |
| Windows | `%APPDATA%\claude-jauge\config.json` |

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
| `-p, --policy NOM` | `fidele` ou `prudent`, pour cet appel seulement. |
| `statusline` | Sous-commande : se fait alimenter par Claude Code. |
| `--no-color` | Aucune couleur (`NO_COLOR` est respecte aussi). |

## Le widget

```sh
cd apps/tray
npm install
npm run tauri dev      # developpement
npm run tauri build    # paquet distribuable
```

* Clic gauche sur l'icone : la popover s'ouvre.
* Clic droit : « Rafraichir maintenant », le choix de la politique, « Quitter ».
* Le titre a cote de l'icone (`96% · 45m`) s'affiche tel quel dans la barre macOS.
  Quand il n'y a rien a montrer il vaut `!` (une raison attend dans la popover)
  ou `...`, jamais un `?` enigmatique.
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

La popover est une fenetre transparente, ce qui sur macOS impose
`app.macOSPrivateApi: true` dans `tauri.conf.json`. Sans ce reglage, Tauri
ignore la transparence et la fenetre s'affiche avec des coins carres opaques
autour du dessin. La contrepartie tient en une ligne : une application qui
utilise ces API ne peut pas etre publiee sur le Mac App Store. Ca ne gene pas
une distribution directe ou par Homebrew, qui est le mode de vie normal d'un
widget de barre de menus. Pour s'en passer, il faudrait renoncer aux coins
arrondis et au flou.

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

Le jeton OAuth est lu la ou Claude Code l'a mis, ou a l'endroit que designe le
bloc `credentials`, et n'est envoye qu'a `api.anthropic.com`. Il n'est jamais
ecrit sur le disque : avec `credentials.command`, il ne fait que transiter en
memoire. Il
n'est jamais reecrit : les jetons de rafraichissement tournent a chaque usage,
donc en consommer un ici risquerait d'invalider la session de Claude Code
elle-meme. Quand le jeton expire, `claude-jauge` le dit et laisse Claude Code le
renouveler.

Les seuls fichiers ecrits sont le cache du dernier releve
(`~/.cache/claude-jauge/state.json` sous Linux) et le fichier de reglages. Ni
l'un ni l'autre ne contient de jeton.

## Integration continue

Le travail est partage entre deux forges, et ce n'est pas un caprice : **un
`.dmg` ne se compile que sur macOS, un `.msi` que sur Windows**. Le serveur
OneDev n'ayant que du Linux, il ne peut pas produire l'app.

| | Quand | Ce qui en sort |
| --- | --- | --- |
| **OneDev** ([`.onedev-buildspec.yml`](.onedev-buildspec.yml)) | chaque push | tests du coeur et du CLI |
| **OneDev** | tag `v*` | binaire CLI Linux, en artefact de build |
| **GitHub Actions** ([`release.yml`](.github/workflows/release.yml)) | tag `v*` | `.dmg` (ARM + Intel), `.msi`, AppImage et `.deb`, plus le CLI des trois OS |

Publier une version :

```sh
git tag -a v0.2.0 -m "claude-jauge 0.2.0"
git push origin v0.2.0    # OneDev : tests + CLI Linux
git push github v0.2.0    # GitHub : les paquets des trois OS
```

La release GitHub est creee **en brouillon** : rien n'est publie sans relecture.

### Deux pieges rencontres

Le buildspec OneDev est un schema versionne, et les exemples qui circulent en
ligne sont perimes. Pour OneDev 16.1.0 il faut `version: 52` (les methodes
`migrateN` du jar amenent *vers* N, donc la derniere presente donne la version
courante, pas celle d'apres), `!PosixInterpreter` et non `DefaultInterpreter`,
`commands` en texte et non en liste, `condition: SUCCESSFUL` et non
`ALL_PREVIOUS_STEPS_WERE_SUCCESSFUL`, `runAs` sous forme `<uid>:<gid>`, et un
`userMatch` obligatoire sur `BranchUpdateTrigger`. Aucun executeur n'a besoin
d'etre configure : OneDev en decouvre un via `docker.sock`.

Cote GitHub, pousser un fichier de workflow exige le scope `workflow` sur le
jeton (`gh auth refresh -h github.com -s workflow`), et **un tag pousse dans le
meme mouvement que la creation du workflow ne le declenche pas** : il faut le
repousser une fois le workflow enregistre.

## Tests

```sh
cargo test        # coeur + CLI, sans serveur graphique
```

84 tests couvrent le parsing des en-tetes (a partir d'un releve reel), la
politique de sonde, le cache, la detection d'activite et le dessin de l'icone.

## Licence

MIT.
