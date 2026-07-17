# TinyXbox360BackupManager

Gestionnaire de sauvegardes de jeux pour **Xbox 360** modifiée (dashboard
[Aurora](https://phoenix.xboxunity.net)), multi-plateformes (Linux, Windows,
macOS), sans aucune dépendance à installer : tout est intégré au binaire.

Inspiré de [TinyWiiBackupManager](https://github.com/mq1/TinyWiiBackupManager) :
comme la Wii lit des jeux Wii et GameCube, la Xbox 360 lit des jeux Xbox 360
et Xbox originale.

## Fonctionnalités

Donnez une image **ISO** à l'application, elle fait le reste :

| Type d'image détecté | Traitement | Destination |
|---|---|---|
| Jeu Xbox 360 (`default.xex`) | Conversion en **GOD** (Games on Demand) | `Content/0000000000000000/<TitleID>/00007000/` |
| Jeu Xbox originale (`default.xbe`) | **Extraction** du contenu | `Games/<Nom du jeu>/` |
| Disque d'installation / DLC (pas d'exécutable) | Extraction et fusion du dossier `Content` | `Content/0000000000000000/<TitleID>/00000002/` |

La bibliothèque se gère **directement sur la cible de votre choix** :

- **Disque USB / dossier local (FAT32)** : les jeux y sont écrits directement
  au bon format.
- **Console en FTP (Aurora)** : la liste des jeux est lue depuis la console,
  les ISO ajoutées sont converties localement puis poussées directement sur
  `Hdd1`, et la suppression s'effectue à distance (une seule connexion à la
  fois, comme l'exige le serveur FTP de la console).
- **Jaquettes** : récupérées automatiquement depuis
  [XboxUnity](https://www.xboxunity.net) (cache local).

La gestion des homebrews n'est volontairement pas couverte.

## Utilisation

L'interface reprend celle de TinyWiiBackupManager : barre latérale (Games,
Toolbox, Settings, About), vue grille ou table, filtres 360/OG, recherche,
file de conversion, notifications, glisser-déposer d'ISO.

1. Cliquer sur l'icône **disque dur** (en bas de la barre latérale) puis
   choisir la cible : **disque USB / dossier local**, ou **console en FTP**
   (IP + identifiants Aurora, `xbox`/`xbox` port 21 par défaut, avec test de
   connexion).
2. Page **Games** : la liste reflète le contenu de la cible (locale ou
   distante). Bouton **+** (ou glisser-déposer) pour ajouter des ISO ;
   l'application détecte le type de chaque image et lance conversion GOD ou
   extraction, directement vers la cible (suivi dans la file de conversion
   et la barre d'état).
3. Sur la console : Aurora > Settings > Content Paths, ajouter
   `Hdd1:\Content\0000000000000000\` (et `Hdd1:\Games\`), Scan Depth 3–4,
   puis relancer un scan.

Pour les jeux **multi-disques avec disque d'installation** (ex. GTA V) : le
disque « Play » se convertit en GOD, le disque d'installation est détecté comme
disque de contenu et son dossier `Content` est fusionné au bon endroit —
donnez simplement les deux ISO à l'application.

## Compilation

```sh
cargo build --release
```

Prérequis de build sous Linux (Debian/Ubuntu/Pop!_OS) :

```sh
sudo apt-get install -y build-essential pkg-config libfontconfig1-dev
```

Le binaire est produit dans `target/release/TinyXbox360BackupManager`.

## Technologies

Pur Rust, aucune dépendance externe à l'exécution :

- [Slint](https://slint.dev) — interface graphique
- [iso2god-rs](https://github.com/iliazeus/iso2god-rs) — conversion ISO → GOD
- [xdvdfs](https://crates.io/crates/xdvdfs) — lecture/extraction des images
  XDVDFS (équivalent d'[extract-xiso](https://github.com/XboxDev/extract-xiso))
- [suppaftp](https://crates.io/crates/suppaftp) — client FTP
- [XboxUnity](https://www.xboxunity.net) — jaquettes et title updates
  (endpoints documentés dans `doc/assets-url.md`)

## Licence

GPL-3.0-only. Basé sur le travail de Manuel Quarneti (TinyWiiBackupManager),
iliazeus (iso2god-rs) et antangelo (xdvdfs).
