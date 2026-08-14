# Journal de collaboration IA — Desdec

Ce document est le point de relais entre les personnes qui contribuent au
projet et les assistants IA (Claude, Codex, etc.). Il consigne les décisions,
les éléments vérifiés et les prochaines actions. Il ne contient pas de
raisonnement interne détaillé ; chaque choix est plutôt expliqué de façon
auditable.

## Règles de travail

- Ne jamais écraser ni refactoriser du code existant sans avoir inspecté son
  intention, ses tests et son historique local.
- Avant une modification : relever le périmètre, les contraintes et les
  dépendances concernées.
- Après une modification : documenter les fichiers touchés, les commandes de
  vérification lancées et leurs résultats.
- Utiliser ce journal et les documents d'architecture comme source de vérité
  entre les sessions et entre assistants.
- L'outil vise exclusivement l'étude de binaires dont l'utilisateur est
  propriétaire ou dispose d'une autorisation explicite d'analyser et de
  modifier.

## Décisions initiales — 2026-08-14

### Vision

Créer **Desdec**, un outil open source de désassemblage, d'analyse statique,
de pseudo-décompilation et de patching pédagogique. Il devra fonctionner sur
Linux, macOS et Windows, conserver une empreinte légère et proposer deux
niveaux d'usage : guidé pour débuter, expert pour approfondir.

### Capacités ciblées

1. Ouvrir et identifier les binaires ELF, PE et Mach-O.
2. Désassembler x86-64 en premier, puis étendre l'architecture par modules.
3. Afficher fonctions, chaînes, imports/exports, sections, références croisées
   et graphe de flot de contrôle.
4. Produire une représentation intermédiaire et du pseudo-code explicable ; ce
   pseudo-code ne prétendra jamais reconstituer le code source exact.
5. Éditer des octets ou des instructions avec prévisualisation, validation et
   export vers un nouveau fichier, sans modification silencieuse de l'original.
6. Ajouter des thèmes et plugins installables avec manifest, permissions et
   compatibilité de version déclarés.

### Orientation technique proposée

- Rust 2024, workspace Cargo organisé par modules.
- Interface native `egui`/`eframe` : rapide au démarrage, portable et adaptée
  à une application pédagogique compacte.
- Cœur indépendant de l'UI : formats, mémoire virtuelle, désassemblage,
  analyse, IR, patching et persistance n'importent pas de code graphique.
- Première cible limitée à ELF/PE/Mach-O x86-64 ; un décompilateur universel
  et multi-architecture viendra seulement après validation du pipeline.
- Licence envisagée : Apache-2.0 ou MIT pour le cœur Rust. Éviter d'incorporer
  directement les composants GPL de Cutter/Iaito ; les proposer au besoin
  comme intégrations externes optionnelles.

### Principes UX

- Le mode débutant explique chaque vue, chaque instruction et chaque saut avec
  des exemples locaux et des avertissements proportionnés.
- Le mode expert expose les mêmes données sans les masquer : hexadécimal,
  désassemblage, CFG, références, annotations, scripts et raccourcis.
- Les changements de binaire sont réversibles au niveau du projet et toujours
  distingués du fichier original.

### Prochaines étapes proposées

1. Écrire l'architecture cible et le contrat de plugin.
2. Créer le workspace Cargo minimal et les tests de lecture ELF/PE/Mach-O.
3. Réaliser une première interface : ouverture, vues hexadécimale et
   désassemblage x86-64, navigation par adresse et annotations.
4. Ajouter un mini-parcours pédagogique fondé sur des crackmes explicitement
   légaux et distribués pour l'apprentissage.

## Format des entrées suivantes

```md
## YYYY-MM-DD — titre court

- Objectif :
- Contexte et hypothèses :
- Décision et justification :
- Fichiers modifiés :
- Vérifications :
- Risques ou limites :
- Suite :
```

## 2026-08-14 — jalon initial exécutable

- Objectif : établir une base de travail légère, portable et indépendante de
  l'interface pour que les futurs apports restent compatibles.
- Contexte et hypothèses : dépôt initialement vide ; la première cible est
  l'identification sûre d'ELF, PE et Mach-O, pas encore le désassemblage.
- Décision et justification : workspace Cargo 2024 avec `desdec-core` sans
  dépendance graphique et `desdec-app` en `egui`/`eframe`. Cette séparation
  protège les analyses et tests des changements d'UX.
- Fichiers modifiés : workspace, application native, cœur de détection,
  documentation et configuration Git.
- Vérifications : `rustfmt --edition 2024` et `cargo fmt --check` réussis ;
  `cargo test --workspace` réussi (3 tests de détection, 0 échec) ;
  `cargo check -p desdec-app` réussi.
- Risques ou limites : les parseurs ne lisent encore que les en-têtes et
  l'application doit télécharger ses dépendances Rust au premier build.
- Suite : valider le jalon, puis ajouter la lecture structurée des sections et
  symboles.

## 2026-08-14 — navigation rétractable et barre d’actions

- Objectif : adopter une ergonomie d’IDE moderne sans encombrer l’ouverture de
  l’application.
- Contexte et hypothèses : les références visuelles sont disponibles dans
  `captures_moi/` ; le menu de navigation doit être replié par défaut.
- Décision et justification : barre d’actions fixe en haut, menu latéral ouvert
  uniquement par le bouton hamburger, barre d’état fixe en bas. Les commandes
  déjà disponibles (ouverture, fermeture, changement de vue, palette) sont
  actives ; les vues futures annoncent clairement leur état.
- Fichiers modifiés : `crates/desdec-app/src/main.rs` et ce journal.
- Vérifications : `cargo fmt --check`, `cargo check -p desdec-app` et
  `cargo test --workspace` réussis ; démarrage et rendu de la fenêtre vérifiés
  sous XWayland, menu effectivement replié au lancement.
- Risques ou limites : la police native ne couvre pas uniformément les
  pictogrammes Unicode. La barre utilise donc des libellés courts garantis
  (ACC, ASM, Fn, STR, PATCH) en attendant un jeu d’icônes embarqué.
- Suite : valider visuellement la fenêtre puis commencer le lecteur de sections.

## 2026-08-14 — préférences et langues intégrées

- Objectif : fusionner les thèmes et extensions dans un point d’entrée clair,
  avec une interface utilisable par défaut en français.
- Décision et justification : « Préférences » remplace « Thèmes et extensions ».
  Les thèmes système, sombre, clair et Catppuccin Mocha sont intégrés ; les
  futurs thèmes communautaires gratuits restent explicitement des extensions.
- Persistance : thème et langue sont enregistrés par le stockage natif
  d’eframe ; le thème système est réappliqué pendant l’exécution pour suivre
  l’OS.
- Internationalisation : toutes les chaînes visibles de l’application passent
  par `i18n.rs`, avec français (défaut), anglais et espagnol.
- Fichiers modifiés : `main.rs`, `i18n.rs`, `preferences.rs`, `Cargo.toml`,
  `Cargo.lock` et ce journal.
- Vérifications : `cargo check -p desdec-app` et `cargo test --workspace`
  réussis (6 tests, dont couverture de toutes les chaînes traduites).
- Suite : valider visuellement les quatre thèmes, puis commencer le lecteur de
  sections.

## 2026-08-14 — position des dialogues traduits

- Problème : le dialogue Préférences changeait de position au changement de
  langue.
- Cause : le titre traduit était aussi l’identifiant implicite de la fenêtre
  egui ; modifier le titre créait un nouvel identifiant et une nouvelle
  géométrie mémorisée.
- Correction : identifiants internes stables pour les dialogues Préférences,
  Palette de commandes et À propos, indépendants de leur titre traduit.

## 2026-08-14 — barre d’outils et commandes configurables

- Objectif : rendre les actions principales immédiatement reconnaissables,
  accessibles au clavier et adaptables sans sacrifier la légèreté.
- Décision et justification : la barre d’outils emploie des icônes vectorielles
  dessinées par l’application, sans police d’icônes ni ressource externe. Son
  affichage est réglable dans Préférences > Comportement ; la barre supérieure
  compacte reste toujours disponible pour la réactiver.
- Commandes : un registre unique alimente la palette (`Ctrl+Maj+P`), les
  boutons, les raccourcis et la page Préférences > Raccourcis. Les conventions
  par défaut couvrent l’ouverture, la fermeture, la navigation, les vues,
  l’aide et le mode guidé/expert ; toute commande est aussi disponible dans la
  palette.
- Personnalisation : les raccourcis peuvent être capturés au clavier, remis à
  zéro et réattribués sans doublon. Lorsqu’une combinaison est réaffectée,
  l’ancienne commande est explicitement désactivée afin d’éviter un conflit.
- Persistance : active par défaut pour les préférences et personnalisations ;
  elle peut être désactivée dans Préférences > Comportement, ce qui efface les
  réglages sauvegardés à la fermeture.
- Fichiers modifiés : `main.rs`, `commands.rs`, `icons.rs`, `i18n.rs`,
  `preferences.rs` et ce journal.
- Vérifications : `cargo fmt --check` et `cargo test --workspace` réussis
  (8 tests, 0 échec). Rendu de la barre d’icônes contrôlé dans la fenêtre
  exécutée sous XWayland.

## 2026-08-14 — contrôle des infobulles

- Objectif : laisser chaque personne adapter la densité d’aide visuelle de
  l’interface.
- Décision : Préférences > Comportement comprend « Afficher les infobulles »,
  activé par défaut et persisté avec les autres réglages. L’option couvre les
  icônes de la barre d’outils ainsi que les boutons qui comportent une aide au
  survol.
- Commande : « Afficher ou masquer les infobulles » est présente dans la
  palette et liée par défaut à `Ctrl+Alt+I`; elle est aussi modifiable dans la
  page Raccourcis.
- Vérifications : `cargo fmt --check` et `cargo test --workspace` réussis
  (8 tests, 0 échec).

## 2026-08-14 — menu fixe et hiérarchisé

- Problème : la poignée de redimensionnement du panneau de navigation était
  trop visible et détournait l’attention du contenu.
- Décision : navigation à largeur fixe de 276 px, sans séparateur manipulable.
  La barre devient plus calme tout en laissant le menu entièrement repliable
  via le hamburger.
- Design : en-tête discret, bouton principal d’ouverture en couleur d’accent,
  action secondaire allégée et sections Exploration / Outils espacées et
  repliables sans lignes horizontales dominantes.
- Vérifications : `cargo fmt --check` et `cargo test --workspace` réussis
  (9 tests, 0 échec) ; capture XWayland du rendu effectuée.

## 2026-08-14 — ouverture non bloquante et builds multiplateformes

- Problème : l’ouverture d’un binaire pouvait faire déclarer l’application
  « ne répond pas ». La boîte de dialogue native était synchrone sur le fil
  graphique et l’inspection lisait le fichier entier avant de le tronquer.
- Correction : sélection de fichier asynchrone, analyse réalisée dans un fil
  de travail et retour vers l’UI par canal. L’application reste donc réactive
  durant l’ouverture. Le cœur lit désormais strictement les 4 Kio d’en-tête
  requis, même pour un binaire très volumineux.
- Vérifications : test de non-lecture complète ajouté ; tests locaux réussis
  (10 tests, 0 échec). `cargo check` a réussi pour `x86_64-pc-windows-msvc` et
  `aarch64-apple-darwin`.
- Livraison : workflow GitHub Actions natif pour une archive Windows x86-64 et
  une archive macOS ARM64/Apple Silicon. Les exécutables signés devront être
  produits et signés sur une infrastructure de publication dédiée.

## 2026-08-14 — palette navigable et contraste discret

- Palette : fenêtre redimensionnable avec dimensions minimales ; le premier
  résultat est sélectionné à l’ouverture ou après une recherche. Les flèches
  haut/bas déplacent cette sélection et `Entrée` exécute explicitement la
  commande en surbrillance.
- Préférences : contours des boutons radio légèrement renforcés pour les
  thèmes sombre et Catppuccin, avec une accentuation supplémentaire seulement
  au survol ou à l’activation.
- Vérifications : `cargo fmt --check`, `cargo check -p desdec-app` et
  `cargo test --workspace` réussis (11 tests, 0 échec).

## 2026-08-14 — refactorisation à comportement constant et thème non sauvegardé

- Objectif : réduire la duplication accumulée dans l'interface et corriger la
  perte du thème signalée sous Windows, sans modifier le rendu ni les réglages
  déjà enregistrés par les utilisateurs.

### Registres générés au lieu d'être recopiés

- Contexte : `Text`, `Command` et `KeyName` maintenaient chacun une liste `ALL`
  écrite à la main à côté de l'énumération, plus une ou deux tables de
  correspondance. Ajouter une entrée d'un seul côté passait inaperçu.
- Décision : trois macros déclaratives (`translations!`, `commands!`, `keys!`)
  produisent l'énumération, la liste ordonnée et les tables depuis une source
  unique. Les traductions sont désormais groupées par clé, français, anglais et
  espagnol sur la même ligne, ce qui rend une langue manquante visible.
- Effet : `i18n.rs` passe de 503 à 261 lignes, `commands.rs` de 490 à 334, et
  l'oubli d'une entrée devient une erreur de compilation.

### Découpage de l'application

- `main.rs` (1019 lignes) ne contient plus que le point d'entrée natif. L'état
  et le cycle de vie vivent dans `app.rs` ; chaque panneau, dialogue et vue a
  son module sous `ui/`, aucun ne conservant d'état propre.
- Les cinq blocs d'icônes recopiés de la barre d'outils deviennent deux tables
  parcourues en boucle. Les dialogues sont regroupés dans `Dialogs`, l'état de
  la palette dans `PaletteState`, les fils de travail dans `BackgroundJobs`.
- `WorkspaceView::planned_explanation` remplace un `unreachable!()` par un
  `Option<Text>` : la vue « Vue d'ensemble » ne peut plus provoquer de panique
  si elle change de statut.

### Thème non sauvegardé

- Cause : `eframe` n'écrit ses données qu'à la fin d'une frame rendue, lors
  d'une sauvegarde automatique espacée de 30 secondes par défaut ou d'un arrêt
  propre. Une fenêtre au repos ne rend aucune frame : un réglage modifié puis
  suivi d'une fermeture brutale, d'une fin de session ou d'un arrêt du système
  n'atteignait jamais le disque. Le symptôme est apparu sous Windows mais la
  fenêtre de perte existait sur les trois plateformes.
- Correction : intervalle ramené à 2 secondes et, surtout, `DesdecApp` compare
  les préférences à celles réellement écrites et demande explicitement une
  frame tant qu'elles diffèrent. La sauvegarde ne dépend donc plus d'une
  fermeture propre. Le stockage n'écrit que si une valeur a changé, donc rien
  n'est sollicité tant que l'utilisateur ne modifie rien.
- `with_app_id("Desdec")` fixe explicitement le dossier de stockage au lieu de
  le laisser dériver du titre de la fenêtre. La valeur est identique à celle
  déduite jusqu'ici : les réglages existants sont conservés.
- Vérification : un test exécute de vraies frames `egui` et contrôle qu'une
  application intacte reste au repos, qu'une préférence modifiée programme une
  frame sous 2 secondes, et que l'écriture ramène l'application au repos.

### Autres points

- `preferences.rs` : les thèmes sombre et Catppuccin partagent une description
  de surfaces au lieu de deux séries d'affectations presque identiques.
- `binary.rs` : les décalages et numéros de machine ELF, PE et Mach-O sont
  nommés dans trois modules dédiés ; `read_u16` et `read_u32` proviennent d'une
  même macro, prête pour le `read_u64` du lecteur de sections.
- `icons.rs` : la fonction de dessin de 126 lignes est découpée en une fonction
  par icône.

- Fichiers modifiés : `main.rs`, `app.rs` (nouveau), `ui/` (nouveau, 8
  modules), `commands.rs`, `i18n.rs`, `icons.rs`, `preferences.rs`,
  `binary.rs`, `desdec-app/Cargo.toml`, `Cargo.lock` et ce journal.
- Vérifications : `cargo fmt --all --check` réussi ; `cargo clippy --workspace
  --all-targets` sans aucun avertissement, contre 29 auparavant ;
  `cargo test --workspace` réussi (32 tests, contre 11) ; `cargo check` réussi
  pour `x86_64-pc-windows-msvc` et `aarch64-apple-darwin` ; `cargo build
  --release` réussi et application lancée sous XWayland, relisant sans erreur
  le `app.ron` écrit par la version précédente.
- Risques ou limites : la sauvegarde reste portée par `eframe` ; un arrêt
  survenant dans les deux secondes qui suivent une modification perdrait encore
  celle-ci. Un stockage écrit directement par l'application supprimerait cette
  limite au prix d'un changement d'emplacement de fichier.
- Suite : le refactor ne change aucun comportement visible ; le lecteur de
  sections reste la prochaine étape.

## 2026-08-14 — module d'analyse approfondie du binaire chargé

- Objectif : passer de l'identification par en-tête à la lecture structurée du
  contenu, et relier ce module aux vues Segments et Chaînes qui n'étaient
  jusqu'ici que des annonces.

### Ce que le module lit

- `desdec-core/src/analysis/` expose `analyse_path` et le type `Analysis` :
  table des sections normalisée pour les trois formats, point d'entrée,
  chaînes lisibles, entropie globale et par section.
- `sections.rs` : parseurs ELF (32/64 bits, petit et gros-boutiste, noms lus
  depuis `.shstrtab`), PE (adresses résolues via l'`ImageBase`, PE32 et PE32+)
  et Mach-O (segments et sections, 32/64 bits, `LC_MAIN`). Les trois produisent
  un même type `Section` : nom, adresse virtuelle, décalage fichier, tailles,
  droits `rwx` et entropie.
- `strings.rs` : extraction ASCII et UTF-16LE. Une chaîne UTF-16 n'est pas
  aussi rapportée comme les fragments ASCII que ses octets nuls produiraient.
- `entropy.rs` : entropie de Shannon en bits par octet. Au-delà de 7,2, le
  contenu ressemble à des données compressées ou chiffrées. C'est un indice
  présenté comme tel, jamais un verdict.

### Sûreté du parseur

Le module lit des fichiers non fiables, éventuellement conçus pour nuire à
l'outil qui les ouvre. Trois propriétés sont tenues et testées :

- **Borné** : lecture plafonnée à 256 Mio, tables de sections à 4096 entrées,
  chaînes à 20 000. Un compteur corrompu ou une commande de chargement de
  taille nulle ne peuvent provoquer ni boucle infinie ni allocation sans fin.
- **Total** : aucune entrée ne panique. Tous les accès passent par les lecteurs
  bornés de `bytes.rs`, qui rendent `None` au lieu d'indexer hors limites. Une
  structure illisible donne une liste vide.
- **Lecture seule** : le fichier est ouvert en lecture et jamais modifié.

### Vérification

- Les fixtures étant écrites à la main, elles peuvent encoder deux fois la même
  erreur de compréhension. Trois vérifications indépendantes les complètent :
  la sortie sur `/usr/bin/ls` a été comparée à `readelf -S` — adresses,
  décalages, tailles et droits identiques pour `.init`, `.plt`, `.text` et
  `.dynsym` ; un vrai binaire PE (`maltego.exe`) a été analysé avec succès,
  `ImageBase` et point d'entrée compris ; un test analyse l'exécutable de test
  lui-même, donc un binaire réel dans le format natif de la plateforme.
- Le parseur Mach-O a révélé un vrai défaut pendant l'écriture des tests : le
  point d'entrée était calculé depuis l'adresse de la première *section* et non
  du *segment* `__TEXT`. Corrigé et couvert.
- `section_at` a été corrigé de même : les sections non allouées (`.shstrtab`,
  `.symtab`) sont stockées à l'adresse 0 et faisaient correspondre n'importe
  quelle recherche d'adresse. `Section::is_mapped` les écarte désormais.

### Interface

- La vue d'ensemble ajoute point d'entrée (et la section qui le contient),
  nombre de sections, nombre de chaînes, entropie et octets analysés.
- « Segments » affiche la table des sections ; une section exécutable dense est
  signalée en couleur avec une explication au survol.
- « Chaînes » affiche les chaînes filtrables. La liste est virtualisée : seules
  les lignes visibles sont mises en page, donc vingt mille chaînes restent
  fluides.
- Un fichier analysé partiellement et une extraction ayant atteint sa limite le
  disent explicitement, plutôt que de paraître complets.
- `desdec-app <binaire>` ouvre directement un fichier, comme le ferait un
  gestionnaire de fichiers.

### Tests de rendu sans fenêtre

L'environnement de développement est en Wayland natif, où la capture d'écran
automatisée n'est pas disponible. Les vues sont donc vérifiées en exécutant de
vraies frames `egui` sans fenêtre : les six vues, dans les trois langues et les
deux modes, avec une analyse réelle et avec aucun binaire chargé. Chaque frame
doit produire des formes à dessiner, ce qui distingue « ne panique pas » de
« affiche réellement quelque chose ».

- Fichiers modifiés : `desdec-core` (`analysis/mod.rs`, `analysis/sections.rs`,
  `analysis/strings.rs`, `analysis/entropy.rs`, `bytes.rs` — tous nouveaux —
  ainsi que `binary.rs` et `lib.rs`), `desdec-app` (`app.rs`, `i18n.rs`,
  `ui/mod.rs`, `ui/views.rs`, `ui/segments.rs` et `ui/strings.rs` nouveaux,
  `ui/status_bar.rs`, `ui/navigation.rs`) et ce journal.
- Vérifications : `cargo fmt --all --check` réussi ; `cargo clippy --workspace
  --all-targets` sans aucun avertissement ; `cargo test --workspace` réussi
  (64 tests, contre 32) ; `cargo check` réussi pour `x86_64-pc-windows-msvc` et
  `aarch64-apple-darwin` ; `cargo build --release` réussi et application lancée
  avec un binaire en argument.
- Risques ou limites : le parseur Mach-O n'a été confronté qu'à des fixtures,
  faute de binaire macOS sur la machine de développement ; ELF et PE l'ont été
  à de vrais fichiers. Les sections ne sont pas encore reliées au futur
  adressage virtuel ni aux symboles.
- Suite : table des symboles et imports/exports, puis désassemblage x86-64 des
  sections exécutables, qui pourront s'appuyer sur `Section::bytes_in` et
  `Analysis::section_at`.

## 2026-08-14 — mode expert : détails de chargement, protections et empreinte

- Objectif : donner un contenu réel au mode expert, qui jusqu'ici ne faisait que
  masquer les aides pédagogiques sans rien ajouter.

### Ce que le mode expert affiche désormais

- **Identité** : type de fichier (exécutable, bibliothèque partagée, objet,
  core, bundle), taille de mot, ordre des octets, chargeur ELF (`PT_INTERP`),
  sous-système et horodatage PE, empreinte SHA-256.
- **Protections** : PIE, pile non exécutable, RELRO (aucun / partiel / complet),
  détection d'écrasement de pile, ASLR, DEP, CFG et présence d'une signature.
- **Bibliothèques liées** : `DT_NEEDED` pour ELF, table d'import pour PE,
  `LC_LOAD_DYLIB` pour Mach-O.
- **Mappage mémoire** : segments de chargement avec adresse, décalage, taille et
  droits — la vue du chargeur, plus grossière que celle des sections.
- La table des sections gagne une colonne « taille en mémoire », qui diffère de
  la taille stockée pour les sections à remplissage nul comme `.bss`.

### Ne pas affirmer plus que ce que le fichier dit

Chaque protection est un tri-état, pas un booléen. `Some(true)`/`Some(false)`
signifient que le format donne la réponse ; `None` signifie que la notion
n'existe pas dans ce format ou que la structure était illisible. L'interface
affiche alors « non applicable » en gris plutôt qu'un « non » rouge : présenter
une inconnue comme une protection absente serait une affirmation de sécurité
non fondée. Deux indications portent explicitement leur réserve au survol :
la détection de canari est déduite des symboles présents, et la présence d'une
signature est constatée sans que sa validité soit vérifiée.

### SHA-256 sans dépendance

`analysis/hash.rs` implémente SHA-256, conformément au principe d'un cœur sans
dépendance. Il est testé contre les vecteurs publiés avec FIPS 180-4, y compris
le message d'un million de caractères et les trois cas de remplissage autour de
la frontière de bloc. L'empreinte est délibérément `None` lorsque le fichier n'a
été lu qu'en partie : le condensat d'un préfixe serait pris pour l'identité du
fichier.

### Vérification

- ELF : la sortie sur `/usr/bin/ls` a été comparée à `readelf -l` et `readelf -d`
  — les treize segments, leurs adresses, tailles et droits sont identiques, les
  trois `DT_NEEDED` correspondent, et `BIND_NOW` donne bien un RELRO complet. Le
  SHA-256 est celui de `sha256sum`. Une bibliothèque partagée (`libz.so.1`) est
  correctement distinguée d'un exécutable PIE : les deux sont `ET_DYN`, seul
  l'exécutable nomme un interpréteur.
- PE : les cinq DLL importées de `maltego.exe` correspondent à sa table
  d'import ; `nbexec.dll`, présente dans les chaînes mais chargée dynamiquement,
  est correctement absente de la liste. Sous-système, horodatage, ASLR et DEP
  lus conformément aux drapeaux du fichier. SHA-256 conforme à `sha256sum`.
- Mach-O : couvert par fixture uniquement, faute de binaire macOS disponible.
- Fichiers modifiés : `desdec-core` (`analysis/details.rs`,
  `analysis/details/{elf,pe,mach_o}.rs` et `analysis/hash.rs` nouveaux ;
  `analysis/mod.rs`, `analysis/sections.rs`, `lib.rs`), `desdec-app`
  (`ui/expert.rs` nouveau ; `i18n.rs`, `ui/mod.rs`, `ui/views.rs`,
  `ui/segments.rs`) et ce journal.
- Vérifications : `cargo fmt --all --check` réussi ; `cargo clippy --workspace
  --all-targets` sans avertissement ; `cargo test --workspace` réussi (72 tests,
  contre 64) ; `cargo check` réussi pour `x86_64-pc-windows-msvc` et
  `aarch64-apple-darwin` ; application lancée avec un binaire en argument.
- Risques ou limites : la détection de canari repose sur les noms de symboles
  visibles et donnera un faux négatif sur un binaire dépouillé et lié
  statiquement ; la table de symboles, prochain jalon, la rendra fiable. Le
  rendu du panneau expert n'a pas pu être contrôlé visuellement (Wayland natif),
  seulement par mise en page réelle sans fenêtre.
- Suite : table des symboles et exports, qui fiabilisera aussi la détection de
  canari.

## 2026-08-14 — titre suivant le mode et mise en page du mode expert

- Objectif : rendre le mode sélectionné visible dans le titre, laisser le mode
  expert occuper la fenêtre entière, et réorganiser les cadres en conséquence.

### Titre

« Desdec / analyse guidée » devient « Desdec / analyse experte » dès que le mode
expert est choisi depuis la barre d'état. Le libellé provient d'un seul
accesseur, `analysis_mode_label`, également utilisé par l'infobulle du bouton de
bascule : les deux ne peuvent plus se contredire. Traduit dans les trois langues.

### Occupation de l'espace

Les zones de défilement de la vue d'ensemble et de la table des sections avaient
`auto_shrink` actif : chaque cadre se réduisait à son contenu, laissant la
fenêtre à moitié vide. Elles s'étendent désormais sur toute la largeur, et un
constructeur de cadre partagé (`ui::card`) force chaque panneau à la largeur
disponible pour qu'ils s'alignent au lieu de flotter à des largeurs différentes.

### Réorganisation des cadres

- Les alertes — code dense, analyse tronquée — passent en tête : elles changent
  la façon de lire tout ce qui suit.
- Le mode expert dispose deux colonnes lorsque la fenêtre dépasse 900 px : à
  gauche ce que le fichier *est* (identité, protections), à droite ce qu'il
  *contient et référence* (analyse, bibliothèques, mappage mémoire). Sous
  900 px, la disposition revient à une colonne. Le tableau du mappage défile
  horizontalement à l'intérieur de sa colonne : ses cinq colonnes sont serrées
  en demi-largeur, et il vaut mieux les faire défiler que déborder du cadre.
- Le cadre « Type de fichier » du mode expert a disparu : ses lignes rejoignent
  le cadre « Fichier actif », qui répétait sinon presque les mêmes informations.
  L'empreinte SHA-256 est placée sous la grille plutôt que dedans, ses
  soixante-quatre caractères déformant sinon la colonne.
- Le mode guidé conserve une colonne unique et l'invitation à passer en expert.

### Vérification

Le test de mise en page sans fenêtre exécute désormais chaque vue à deux
largeurs, de part et d'autre du seuil de bascule, afin que les deux dispositions
soient réellement parcourues et non pas seulement celle que retient egui par
défaut. Un test distinct vérifie que le libellé d'analyse et le libellé court
suivent le mode dans les trois langues.

- Fichiers modifiés : `i18n.rs`, `app.rs`, `ui/mod.rs`, `ui/views.rs`,
  `ui/expert.rs`, `ui/action_bar.rs`, `ui/status_bar.rs`, `ui/segments.rs` et ce
  journal.
- Vérifications : `cargo fmt --all --check` réussi ; `cargo clippy --workspace
  --all-targets` sans avertissement ; `cargo test --workspace` réussi (73 tests,
  contre 72) ; `cargo check` réussi pour `x86_64-pc-windows-msvc` et
  `aarch64-apple-darwin` ; application lancée avec un binaire en argument.
- Risques ou limites : le seuil de 900 px et la répartition en deux colonnes
  égales sont des choix de conception faits sans contrôle visuel possible
  (Wayland natif). Ils demandent un avis humain.

## 2026-08-15 — retrait de « Aucun correctif appliqué »

- Problème : la barre d'état affichait en permanence « Aucun correctif
  appliqué », c'est-à-dire un état permanent portant sur une fonctionnalité qui
  n'existe pas encore. Le message occupait de la place sans jamais rien
  apprendre, et aurait continué à ne rien dire une fois les correctifs
  implémentés.
- Décision : suppression du libellé, de son séparateur et de sa traduction. La
  barre d'état ne mentionnera les correctifs que lorsqu'il y en aura, et
  seulement à ce moment-là : un compteur ne s'affiche que s'il compte quelque
  chose. Un commentaire à l'emplacement concerné consigne cette règle pour le
  jalon de patching.
- Fichiers modifiés : `ui/status_bar.rs`, `i18n.rs` et ce journal.
- Vérifications : `cargo fmt --all --check` réussi ; `cargo clippy --workspace
  --all-targets` sans avertissement ; `cargo test --workspace` réussi
  (73 tests) ; application lancée avec un binaire en argument.
