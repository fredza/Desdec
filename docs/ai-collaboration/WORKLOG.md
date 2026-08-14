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
