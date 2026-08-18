# Desdec

[English](README.md) · **Français** · [Español](README.es.md)

Desdec est un explorateur de binaires local et open source, fait pour lire les
exécutables qu'on a le droit de lire. Il ouvre un fichier ELF, PE ou Mach-O,
dit ce qu'il contient, et ne l'exécute jamais.

Sa règle de conduite est de ne rien inventer. Quand une réponse est exacte —
l'adresse que désigne un opérande, les octets qu'un correctif écrirait — elle
est donnée telle quelle. Quand c'est une lecture locale qu'un branchement peut
invalider, il le dit. Quand il ne sait pas, il le dit aussi plutôt que de
deviner.

> N'analysez et ne modifiez que des binaires qui vous appartiennent ou que vous
> êtes explicitement autorisé à étudier.

![La vue Désassemblage, avec le pseudo-code local à côté](docs/screenshots/disassembly.png)

## Ce qu'il montre

| Vue | Ce qu'on y trouve |
| --- | --- |
| **Aperçu** | Format, architecture, point d'entrée, SHA-256, entropie, durcissement (RELRO, canari, NX, PIE, CFG), langage source détecté, et chaque bibliothèque liée — avec l'explication de ce à quoi elle sert. |
| **Segments** | La table des sections : adresses, tailles, permissions et entropie par section, pour qu'une zone compressée ou chiffrée saute aux yeux. |
| **Fonctions** | Les fonctions nommées, leur corps, leurs blocs de base et un graphe de flot de contrôle local. |
| **Chaînes** | Les chaînes imprimables avec leur décalage et leur encodage, filtrables, et les instructions qui les référencent. |
| **Désassemblage** | Listings x86, x86-64 (iced-x86) et AArch64 (Capstone), avec édition des octets d'une instruction. Un clic droit explique ce que désigne l'opérande et ce qui a écrit en dernier dans chaque registre nommé. |
| **Pseudo-code** | Une traduction prudente du flot décodé, intégrée à l'outil — ou la sortie de Rizin/rz-ghidra ou de RetDec si l'un d'eux est installé et choisi. |
| **Correctifs** | Les modifications d'octets en attente, et l'export qui les écrit dans une **copie**. Le fichier analysé n'est jamais modifié. |
| **YARA** | Optionnel. Lance un `yara` ou `yr` installé localement sur le fichier ouvert, avec vos propres règles. Désactivé par défaut. |
| **Assistance IA** | Optionnelle, désactivée par défaut. Un modèle relit ce qui a été décodé — un binaire entier, une fonction, une instruction — et sa réponse est étiquetée comme une lecture proposée, jamais comme un constat. Un modèle local (Ollama) ou l'API d'Anthropic, selon ce que vous configurez. |

Tout est disponible en français, en anglais et en espagnol, depuis une palette
de commandes (`Ctrl+Maj+P`) dont les raccourcis sont réassignables.

## Captures d'écran

**Avant d'ouvrir un fichier.** Le menu garde les fichiers récents et les vues ;
la barre d'actions reste disponible, que le menu soit ouvert ou replié.

![L'état vide, le menu de navigation ouvert](docs/screenshots/start.png)

**Fonctions.** Les fonctions nommées avec leur taille et leur nombre de blocs,
le graphe de flot de contrôle local de celle qui est sélectionnée, et son
pseudo-code en dessous.

![La vue Fonctions : la liste, un graphe de flot de contrôle et du pseudo-code](docs/screenshots/functions.png)

**Chaînes.** Chaque chaîne imprimable avec son décalage et son encodage,
filtrable, et réductible à celles qui ne sont pas mappées ou jamais
référencées.

![La vue Chaînes, avec son filtre et ses deux restrictions](docs/screenshots/strings.png)

**Décompilateur externe.** Rizin avec rz-ghidra, ou RetDec, quand l'un d'eux
est installé et choisi — le moteur qui a produit le texte est toujours nommé,
et le désassemblage correspondant est à un bouton de là.

![Du pseudo-code produit par rizin et rz-ghidra, le moteur nommé au-dessus](docs/screenshots/decompile.png)

**Correctifs.** Les modifications d'octets attendent ici jusqu'à l'export, et
l'export écrit une copie : le fichier analysé n'est jamais modifié.

![La vue Correctifs, vide, expliquant d'où viennent les modifications](docs/screenshots/patches.png)

**Palette de commandes** (`Ctrl+Maj+P`). Toutes les commandes, leur raccourci
et les fichiers récemment ouverts, dans une seule liste cherchable.

![La palette de commandes, listant les commandes et leurs raccourcis](docs/screenshots/command-palette.png)

**Préférences.** Les moteurs externes sont cherchés dans le `PATH` ou pointés
par un chemin à vous, et ne sont lancés qu'une fois l'un d'eux sélectionné.

![La fenêtre Préférences, sur son onglet Décompilateur](docs/screenshots/preferences.png)

## Installer et lancer

Rust 1.85 ou plus récent.

```sh
git clone https://github.com/fredza/Desdec.git
cd Desdec
cargo run --release -p desdec-app            # ouvrir la fenêtre
cargo run --release -p desdec-app -- /bin/ls # ou analyser un fichier tout de suite
```

On peut aussi déposer un binaire sur la fenêtre, ou utiliser **Ouvrir un
binaire** (`Ctrl+O`).

Des archives précompilées pour Windows x86-64, macOS Apple Silicon et Linux
x86-64 sont publiées par le workflow `Platform binaries` à chaque étiquette
commençant par `v`, avec leurs sommes SHA-256.

## Ce qu'il fait de vos fichiers et de votre machine

- **Il n'exécute jamais le binaire analysé.** Rien n'en est lancé, ni mappé, ni
  chargé.
- **Il lit, et n'écrit que là où vous le demandez.** Le fichier analysé est
  ouvert en lecture seule ; un correctif est écrit dans une copie distincte que
  vous nommez vous-même.
- **Il n'établit aucune connexion réseau tant que vous n'en configurez pas
  une.** Tel qu'il sort de sa boîte, il ne se connecte à rien. L'assistance IA
  optionnelle en est la seule exception, et seulement après avoir choisi un
  fournisseur : un modèle local sur la boucle locale, ou l'API d'Anthropic sur
  Internet. Même alors, ce sont les faits extraits — instructions, noms de
  symboles, chaînes — qui partent, jamais le fichier, et la vue montre le texte
  exact avant que vous demandiez quoi que ce soit.
- **Chaque octet exécutable lu est décodé** : il n'y a aucun plafond sur le
  nombre d'instructions. Une grande bibliothèque partagée en atteint réellement
  dix-huit millions, et la liste est virtualisée : sa longueur ne coûte rien à
  l'interface.
- Ce qui reste borné, c'est la lecture : au plus 256 Mio par fichier, 20 000
  chaînes, 4 096 entrées de section. Quand une limite est atteinte, l'interface
  le dit, au lieu de présenter une liste partielle comme si c'était tout le
  programme.
- Les seuls programmes externes qu'il démarre sont ceux que vous choisissez :
  un décompilateur (`rizin`, `retdec-decompiler`), YARA, ou un serveur de
  modèle local. Aucun n'est requis, aucun n'est lancé sans avoir été
  sélectionné dans les préférences.
- **Une clé d'API n'est jamais écrite dans le fichier de préférences.** La clé
  Anthropic est lue depuis `ANTHROPIC_API_KEY`, ou depuis un fichier que vous
  désignez et dont les permissions vous appartiennent.

### Où il range ses affaires

| | Préférences | Décompilations en cache |
| --- | --- | --- |
| Linux | `$XDG_DATA_HOME/desdec/app.ron` ou `~/.local/share/desdec/app.ron` | `$XDG_CACHE_HOME/desdec/decompiled` ou `~/.cache/desdec/decompiled` |
| macOS | `~/Library/Application Support/Desdec/app.ron` | `~/Library/Caches/desdec/decompiled` |
| Windows | `%APPDATA%\Desdec\data\app.ron` | `%LOCALAPPDATA%\desdec\decompiled` |

Les préférences sont écrites une fraction de seconde après avoir cessé de
changer, et poussées sur le disque à ce moment-là — sans attendre une
sauvegarde périodique ni une fermeture propre. Une fenêtre fermée brutalement
sous Windows perdait le thème choisi quelques instants plus tôt ; ce n'est plus
le cas. La persistance peut être désactivée entièrement, ce qui efface aussi ce
qui était déjà enregistré. Les décompilations sont mises en cache sous le
SHA-256 du fichier dont elles viennent : un fichier tronqué, qui n'a pas
d'empreinte digne de confiance, n'est jamais mis en cache.

## Développement

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```

La suite de tests tourne en une vingtaine de secondes et n'exige rien
d'installé. Elle analyse des binaires ELF, PE et Mach-O AArch64 synthétiques,
forgés octet par octet dans `desdec-core::fixtures` : les lecteurs des formats
absents de la machine hôte sont donc exercés à chaque exécution, sur toutes les
plateformes.

Pour revoir le jeu d'icônes après avoir modifié un glyphe :

```sh
DESDEC_ICON_SHEET=/tmp/icons.svg cargo test -p desdec-app icon_sheet
```

### Organisation

- `crates/desdec-core` — inspection et analyse des binaires. Ne sait rien
  d'aucune interface. La lecture d'entrées non fiables est bornée et totale :
  chaque lecture passe par des accesseurs vérifiés, chaque parcours de table
  est plafonné, et aucune entrée ne peut provoquer de panique.
- `crates/desdec-app` — l'application native `egui`.
- `docs/ARCHITECTURE.md` — le sens des dépendances et ce qui est délibérément
  hors du cœur.
- `docs/ai-collaboration/WORKLOG.md` — les règles de travail communes aux
  contributeurs humains et aux assistants IA.

## Licence

Apache-2.0 OU MIT, au choix : [LICENSE-APACHE](LICENSE-APACHE) et
[LICENSE-MIT](LICENSE-MIT). Les deux sont également accessibles depuis la
fenêtre À propos, afin que les termes soient atteignables depuis l'application.

Sauf mention contraire de votre part, toute contribution que vous soumettez
délibérément pour inclusion dans ce travail sera doublement licenciée comme
ci-dessus, sans terme ni condition supplémentaire.
