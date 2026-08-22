# Desdec

[English](README.md) · **Français** · [Español](README.es.md)

Les versions release et pré-release sont signées avec une clef privée, c'est obligatoire actuellement.
La clef publique est distribuée gratuitement avec le binaire.  

Desdec est un explorateur de binaires local et open source, fait pour lire les
exécutables qu'on a le droit de lire. Il ouvre un fichier ELF, PE ou Mach-O,
dit ce qu'il contient, et ne l'exécute jamais sur votre machine.

Là où il exécute un binaire, c'est sur un processeur qu'il construit lui-même :
un émulateur sans système d'exploitation derrière lui, décrit sous **Machine**
plus bas. Aucun octet du fichier n'atteint jamais votre propre processeur.

Sa règle de conduite est de ne rien inventer. Quand une réponse est exacte —
l'adresse que désigne un opérande, les octets qu'un correctif écrirait — elle
est donnée telle quelle. Quand c'est une lecture locale qu'un branchement peut
invalider, il le dit. Quand il ne sait pas, il le dit aussi plutôt que de
deviner.

> N'analysez et ne modifiez que des binaires qui vous appartiennent ou que vous
> êtes explicitement autorisé à étudier.

![La vue Désassemblage : le listing, les drapeaux de l'instruction sélectionnée dans la barre, et le pseudo-code local à côté](docs/screenshots/disassembly.png)

## Ce qu'il montre

| Vue | Ce qu'on y trouve |
| --- | --- |
| **Aperçu** | Format, architecture, point d'entrée, SHA-256, entropie, durcissement (RELRO, canari, NX, PIE, CFG), langage source détecté, et chaque bibliothèque liée — avec l'explication de ce à quoi elle sert. |
| **Segments** | La table des sections : adresses, tailles, permissions et entropie par section, pour qu'une zone compressée ou chiffrée saute aux yeux. |
| **Fonctions** | Les fonctions nommées, leur corps, leurs blocs de base et un graphe de flot de contrôle local. Un clic sur une ligne ouvre le code de cette fonction dans le listing. Un fichier qui n'en nomme aucune a quand même cette vue : ses fonctions sont trouvées à partir de son propre code — le point d'entrée, tout ce que quelque chose appelle, les prologues de compilateur — et chaque ligne dit d'où elle vient, parce qu'une adresse appelée est un fait et un prologue une lecture. À côté de chacune : ce qui l'appelle, ce qu'elle appelle, et les chaînes d'appels les plus courtes qui y mènent depuis un point de départ du fichier — la question « comment on arrive ici ? », à laquelle ni un listing ni une liste de références ne répondent seuls. |
| **Chaînes** | Les chaînes imprimables avec leur décalage et leur encodage, filtrables, et les instructions qui les référencent. |
| **Désassemblage** | Listings x86, x86-64 (iced-x86) et AArch64 (Capstone), avec édition des octets d'une instruction. Un clic droit explique ce que désigne l'opérande et ce qui a écrit en dernier dans chaque registre nommé. La barre porte les drapeaux de condition de la ligne sélectionnée — ceux qu'elle établit, ceux qu'elle consulte, et ceux dont les octets fixent la valeur quoi qu'il se soit passé avant — et une ligne sur laquelle vous avez écrit est marquée en marge. |
| **Pseudo-code** | Une traduction prudente du flot décodé, intégrée à l'outil — ou la sortie de Rizin/rz-ghidra ou de RetDec si l'un d'eux est installé et choisi. |
| **Machine** | Un processeur émulé, éteint tant que vous n’en demandez pas un. Registres, mémoire, pile, points d’arrêt, surveillances, pas à pas détaillé/principal/sortant, exécution jusqu’au curseur et pile des appels — autant de mesures, puisque quelque chose s’est réellement exécuté. Cela tourne sur un processeur que Desdec construit, jamais sur le vôtre : aucun octet du fichier n’atteint le processeur de votre machine. À un appel système, la vue donne un relevé de type `strace` (ABI, numéro, nom fiable et registres d’arguments) sans lancer l’appel ni lui inventer de résultat ; Linux x86/x86-64, macOS x86-64 et Windows x86-64 sont distingués. Une bibliothèque absente ou une instruction non émulée arrêtent aussi la course et sont nommées plutôt que devinées. x86 et x86-64. Les registres XMM sont visibles, et les mouvements SSE 128 bits usuels (`movaps`, `movups`, `movdqa`, `movdqu`) ainsi que les XOR (`pxor`, `xorps`) s’exécutent avec leur état exact, y compris en reculant ; les instructions YMM/ZMM plus larges restent arrêtées et nommées. Les points d'arrêt portent des conditions (`rcx == 4`, `[rdi]:1 != 0`) et un nombre de passages à laisser filer, de sorte qu'un point d'arrêt dans une boucle de dix mille tours vaut la peine d'être posé. Les emplacements du cadre — ce qu'un débogueur appelle les variables locales — sont lus du code de la fonction où la course s'est arrêtée : chaque `-0x14(%rbp)` et chaque `0x8(%rsp)` qu'elle touche, avec sa largeur, le nombre de lectures et d'écritures, et ce que la course y a effectivement mis. Et la course va **en arrière** : l'état d'avant chaque instruction est conservé, donc reculer le restaure exactement — y compris pour sortir d'une faute, ce qu'un débogueur attaché à un processus ne peut pas faire du tout. |
| **Graphe** | Une fonction dessinée comme son flot de contrôle : ses blocs de base, et les flèches entre eux avec leur raison — la branche prise, celle qui ne l'est pas, un saut, la suite du listing. Un `ret` va quelque part de parfaitement connu et n'a donc pas de flèche ; un saut par registre n'en a pas non plus, et c'est dit autrement, parce que les deux ne sont pas la même chose. |
| **Structures** | Ce que veulent dire les octets à une adresse. Un fichier ne dit presque rien de ses données : le listing écrit `mov 0x18(%rbx),%rax`, et ce que sont ces huit octets est votre savoir, pas le sien. Écrivez-le une fois en C — structures, unions, énumérations, `typedef`, pointeurs, tableaux, champs de bits ; un en-tête se colle tel quel — et il s'applique sur la mémoire de la Machine quand elle tourne, sur les octets du fichier sinon. La disposition est calculée contre la forme du fichier ouvert, y compris le `long` de quatre octets qu'un PE 64 bits emploie là où un ELF en emploie huit. Les structures du format du fichier — `Elf64_Ehdr`, `IMAGE_DOS_HEADER`, `mach_header_64` et ce qui va avec — s'ajoutent d'un bouton, et se lisent au décalage du fichier, seule façon d'atteindre un en-tête qu'aucune section ne mappe. Et une structure se **déduit du code qui la parcourt** : chaque `0x18(%rbx)` d'une fonction est un membre à cet endroit, le reste est nommé comme remplissage, et ce que le code ne dit pas — la longueur d'un tableau, la largeur d'un accès — est rapporté à part plutôt qu'inventé. |
| **Correctifs** | Les modifications d'octets en attente, et l'export qui les écrit dans une **copie**. Le fichier analysé n'est jamais modifié. |
| **Mises à jour** | Optionnelles, et éteintes tant que vous n'avez rien dit. Desdec peut demander à GitHub s'il existe une version plus récente ; la question est posée une fois, et ses réponses sont « oui » et « pas cette fois » — l'éteindre pour de bon se fait dans les préférences. Un téléchargement est comparé à l'empreinte `.sha256` que publie la release, et refusé s'il n'y correspond pas. Desdec ne se remplace jamais lui-même : l'archive arrive dans un dossier, et vous l'ouvrez quand vous le voulez. |
| **YARA** | Optionnel. Lance un `yara` ou `yr` installé localement sur le fichier ouvert, avec vos propres règles. Désactivé par défaut. |
| **Assistance IA** | Optionnelle, désactivée par défaut. Un modèle relit ce qui a été décodé — un binaire entier, une fonction, une instruction — et sa réponse est étiquetée comme une lecture proposée, jamais comme un constat. Un modèle local (Ollama) ou l'API d'Anthropic, selon ce que vous configurez. |
| **Script** | La règle du lecteur, écrite une fois et passée sur tout le fichier : nommer chaque fonction plus longue qu'une page, marquer chaque appel vers une bibliothèque, trouver ce vers quoi le listing ne défilera pas. Elle s'exécute dans un bac à sable sans système de fichiers, sans réseau et sans processus — rien que l'analyse qu'on lui confie. |
| **Greffons** | Un script écrit par quelqu'un d'autre, installé sous forme de dossier avec un manifeste. Ce manifeste *demande* des permissions — écrire des notes, déplacer le listing, proposer des correctifs — et la liste vous est présentée avant toute activation. Un greffon jamais activé n'a jamais été exécuté. |

Tout est disponible en français, en anglais et en espagnol, depuis une palette
de commandes (`Ctrl+Maj+P`) dont les raccourcis sont réassignables.

## Captures d'écran

**Avant d'ouvrir un fichier.** Le menu garde les fichiers récents et les vues ;
la barre d'actions reste disponible, que le menu soit ouvert ou replié.

![L'état vide, le menu de navigation ouvert](docs/screenshots/start.png)

**Fonctions.** Les fonctions nommées avec leur taille et leur nombre de blocs,
le graphe de flot de contrôle local de celle qui est sélectionnée, et son
pseudo-code en dessous. La flèche en tête d'une ligne — ou le bouton à côté de
l'adresse — ouvre le code de cette fonction dans le listing.

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

![La vue Correctifs, avec un correctif en attente et l'export qui écrit une copie](docs/screenshots/patches.png)

**Palette de commandes** (`Ctrl+Maj+P`). Toutes les commandes, leur raccourci
et les fichiers récemment ouverts, dans une seule liste cherchable.

![La palette de commandes, listant les commandes et leurs raccourcis](docs/screenshots/command-palette.png)

**Préférences.** Les moteurs externes sont cherchés dans le `PATH` ou pointés
par un chemin à vous, et ne sont lancés qu'une fois l'un d'eux sélectionné.

![La fenêtre Préférences, sur son onglet Décompilateur](docs/screenshots/preferences.png)

## Installer et lancer

Le script d'installation télécharge l'archive publiée pour votre machine,
vérifie son SHA-256 *et* sa signature, et ce n'est qu'ensuite qu'il pose le
binaire. Sur Linux et macOS (Apple Silicon) :

```sh
curl -fsSL https://raw.githubusercontent.com/fredza/Desdec/main/scripts/install.sh -o install.sh
less install.sh   # il est court, et vous allez l'exécuter
bash install.sh   # installe dans ~/.local/bin
```

Sur Windows (x86-64), le même script en PowerShell — aucun shell POSIX requis :

```powershell
irm https://raw.githubusercontent.com/fredza/Desdec/main/scripts/install.ps1 -OutFile install.ps1
notepad install.ps1   # il est court, et vous allez l'exécuter
.\install.ps1        # installe dans %LOCALAPPDATA%\Programs\Desdec
```

Les deux acceptent `--version` / `-Version v0.3.36` pour une version précise,
`--prefix` / `-Prefix` pour installer ailleurs, et `--from-source` /
`-FromSource` pour compiler sur place ; `--help` et `Get-Help .\install.ps1`
donnent le reste. Une version dont la somme ou la signature ne correspond pas
est jetée, et non installée avec un avertissement au-dessus. Vérifier une
signature demande `gpg` — Gpg4win sous Windows — et sans lui le script
s'arrête plutôt que d'installer ce qu'il n'a pas pu vérifier.

### Depuis les sources

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

### Vérifier une version publiée

Chaque archive est signée par **Frédéric Zawalski @2026 bdom**, avec la clef
`C9A3 1D07 46E0 65C4 E2EA  33F6 08FA 1D81 8A91 F329`. La clef publique voyage
avec les binaires : elle est jointe à chaque release sous le nom
`desdec-signing-key.asc`, et se trouve aussi à la racine du dépôt.

```sh
gpg --import desdec-signing-key.asc
gpg --verify desdec-linux-x86_64-release.tar.gz.asc \
             desdec-linux-x86_64-release.tar.gz
```

La somme SHA-256 répond à une autre question : elle dit que le téléchargement
est intact, pas qui l'a produit. La signature dit les deux. La clef privée ne
quitte jamais la machine du mainteneur ; le service de compilation ne la voit
pas, il ne fait que compiler.

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
- **Un script n'atteint rien d'autre que l'analyse.** Le moteur de script
  reçoit le binaire décodé et les notes prises dessus ; ni système de
  fichiers, ni réseau, ni processus ne figurent dans son vocabulaire — non par
  une règle qu'on pourrait oublier, mais parce que rien de tel n'y a jamais
  été inscrit. Un script venu d'ailleurs s'exécute avec exactement les
  permissions que vous lui avez accordées, et celui dont le manifeste se met à
  en demander davantage s'arrête tant que vous n'avez pas vu la nouvelle liste.
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

Deux autres dossiers vous appartiennent plutôt qu'à l'application, et aucun des
deux n'est un cache : les notes prises sur un binaire vivent sous
`desdec/notes`, un fichier par binaire nommé d'après son SHA-256 plutôt que
d'après son chemin, et les greffons vivent sous `desdec/plugins`, un dossier
chacun. La fenêtre des greffons affiche le chemin exact sur votre machine, et
`examples/plugins` dans ce dépôt en contient un à y copier.

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

## Licence

Apache-2.0 OU MIT, au choix : [LICENSE-APACHE](LICENSE-APACHE) et
[LICENSE-MIT](LICENSE-MIT). Les deux sont également accessibles depuis la
fenêtre À propos, afin que les termes soient atteignables depuis l'application.

Sauf mention contraire de votre part, toute contribution que vous soumettez
délibérément pour inclusion dans ce travail sera doublement licenciée comme
ci-dessus, sans terme ni condition supplémentaire.
