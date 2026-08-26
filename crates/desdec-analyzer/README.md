# `desdec-analyzer`

Analyseur externe de binaires pour Desdec et les outils qui veulent consommer
ses résultats sans dépendre de Rust.

## Contrat d'exécution

```text
desdec-analyzer report <binaire> [--pretty] [--instructions]
```

Le rapport JSON est écrit sur la sortie standard ; les diagnostics sont écrits
sur la sortie d'erreur. Le code de retour est nul uniquement quand le rapport
est complet et syntaxiquement valide.

`protocol_version` est actuellement `1`. Un consommateur accepte sa version
majeure connue et refuse une version plus récente au lieu d'interpréter un
nouveau champ comme une ancienne donnée.

Le rapport comprend notamment :

- identification ELF, PE ou Mach-O, architecture, taille et SHA-256 ;
- point d'entrée, sections, segments, droits mémoire et entropie ;
- protections de compilation/liaison (PIE, NX, RELRO, canari, ASLR, DEP, CFG,
  signature) ;
- chargeur, dépendances, imports et slots de relocation ;
- symboles, classes C++, chaînes, indices de langage et indices réseau ;
- compteur d'instructions et, avec `--instructions`, le désassemblage complet.

Les analyses sont bornées par `desdec-core` et le JSON dit explicitement si le
fichier ou le listing a été tronqué. Une absence d'information est `null` ;
une collection sans élément est `[]`.

## Remplacement dans un autre langage

Le programme configuré dans Desdec ne doit pas nécessairement être ce binaire
Rust. Une implémentation C, C++ ou mixte peut le remplacer si elle accepte la
commande `report <binaire> --pretty` et produit le même JSON versionné sur
stdout. Elle reste un processus séparé : l'application appelante ne charge
jamais une DLL ou une bibliothèque native provenant de l'analyseur.
