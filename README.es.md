# Desdec

[English](README.md) · [Français](README.fr.md) · **Español**

Desdec es un explorador de binarios local y de código abierto, hecho para leer
los ejecutables que uno tiene derecho a leer. Abre un archivo ELF, PE o Mach-O,
dice qué contiene, y nunca lo ejecuta.

Su regla de conducta es no inventar nada. Cuando una respuesta es exacta —la
dirección que designa un operando, los bytes que escribiría un parche— se da
tal cual. Cuando es una lectura local que una bifurcación puede invalidar, lo
dice. Cuando no lo sabe, también lo dice, en lugar de adivinar.

> Analice y modifique únicamente binarios que le pertenezcan o que esté
> explícitamente autorizado a estudiar.

![La vista Desensamblado, con el pseudocódigo local al lado](docs/screenshots/disassembly.png)

## Lo que muestra

| Vista | Lo que se encuentra |
| --- | --- |
| **Resumen** | Formato, arquitectura, punto de entrada, SHA-256, entropía, endurecimiento (RELRO, canario, NX, PIE, CFG), lenguaje de origen detectado, y cada biblioteca enlazada —con la explicación de para qué sirve. |
| **Segmentos** | La tabla de secciones: direcciones, tamaños, permisos y entropía por sección, para que una zona comprimida o cifrada salte a la vista. |
| **Funciones** | Las funciones con nombre, su cuerpo, sus bloques básicos y un grafo de flujo de control local. |
| **Cadenas** | Las cadenas imprimibles con su desplazamiento y su codificación, filtrables, y las instrucciones que las referencian. |
| **Desensamblado** | Listados x86, x86-64 (iced-x86) y AArch64 (Capstone), con edición de los bytes de una instrucción. Con el botón derecho se explica qué designa el operando y qué escribió por última vez en cada registro nombrado. |
| **Pseudocódigo** | Una traducción prudente del flujo decodificado, integrada en la herramienta —o la salida de Rizin/rz-ghidra o de RetDec si alguno está instalado y elegido. |
| **Parches** | Las modificaciones de bytes pendientes, y la exportación que las escribe en una **copia**. El archivo analizado nunca se modifica. |
| **YARA** | Opcional. Ejecuta un `yara` o `yr` instalado localmente sobre el archivo abierto, con sus propias reglas. Desactivado por defecto. |

Todo está disponible en francés, inglés y español, desde una paleta de comandos
(`Ctrl+Mayús+P`) cuyos atajos se pueden reasignar.

## Capturas de pantalla

Aquí la interfaz está en francés; el inglés y el español están a una
preferencia de distancia.

**Antes de abrir un archivo.** El menú conserva los archivos recientes y las
vistas; la barra de acciones sigue disponible, esté el menú abierto o plegado.

![El estado vacío, con el menú de navegación abierto](docs/screenshots/start.png)

**Funciones.** Las funciones con nombre, su tamaño y su número de bloques, el
grafo de flujo de control local de la seleccionada, y su pseudocódigo debajo.

![La vista Funciones: la lista, un grafo de flujo de control y pseudocódigo](docs/screenshots/functions.png)

**Cadenas.** Cada cadena imprimible con su desplazamiento y su codificación,
filtrable, y reducible a las que no están mapeadas o nunca se referencian.

![La vista Cadenas, con su filtro y sus dos restricciones](docs/screenshots/strings.png)

**Decompilador externo.** Rizin con rz-ghidra, o RetDec, cuando alguno está
instalado y elegido —el motor que produjo el texto siempre se nombra, y el
desensamblado correspondiente está a un botón.

![Pseudocódigo producido por rizin y rz-ghidra, con el motor nombrado encima](docs/screenshots/decompile.png)

**Parches.** Las modificaciones de bytes esperan aquí hasta la exportación, y
la exportación escribe una copia: el archivo analizado nunca se modifica.

![La vista Parches, vacía, explicando de dónde vienen las modificaciones](docs/screenshots/patches.png)

**Paleta de comandos** (`Ctrl+Mayús+P`). Todos los comandos, su atajo y los
archivos abiertos recientemente, en una sola lista buscable.

![La paleta de comandos, con los comandos y sus atajos](docs/screenshots/command-palette.png)

**Preferencias.** Los motores externos se buscan en el `PATH` o se apuntan con
una ruta propia, y solo se ejecutan una vez que se selecciona uno.

![La ventana Preferencias, en su pestaña Decompilador](docs/screenshots/preferences.png)

## Instalar y ejecutar

Rust 1.85 o posterior.

```sh
git clone https://github.com/fredza/Desdec.git
cd Desdec
cargo run --release -p desdec-app            # abrir la ventana
cargo run --release -p desdec-app -- /bin/ls # o analizar un archivo de inmediato
```

También se puede arrastrar un binario a la ventana, o usar **Abrir un binario**
(`Ctrl+O`).

El flujo de trabajo `Platform binaries` publica archivos precompilados para
Windows x86-64, macOS Apple Silicon y Linux x86-64 en cada etiqueta que empieza
por `v`, junto con sus sumas SHA-256.

## Qué hace con sus archivos y con su máquina

- **Nunca ejecuta el binario analizado.** Nada de él se lanza, ni se mapea, ni
  se carga.
- **Lee, y solo escribe donde usted lo pide.** El archivo analizado se abre en
  modo lectura; un parche se escribe en una copia aparte que usted mismo
  nombra.
- **No establece ninguna conexión de red.**
- **Cada byte ejecutable leído se decodifica**: no hay tope alguno en el
  número de instrucciones. Una biblioteca compartida grande alcanza realmente
  dieciocho millones, y el listado está virtualizado: su longitud no le cuesta
  nada a la interfaz.
- Lo que sigue acotado es la lectura: como máximo 256 MiB por archivo, 20 000
  cadenas, 4 096 entradas de sección. Cuando se alcanza un límite, la interfaz
  lo dice, en lugar de presentar un listado parcial como si fuera todo el
  programa.
- Los únicos programas externos que inicia son los que usted elige: un
  descompilador (`rizin`, `retdec-decompiler`) o YARA. Ninguno es obligatorio,
  y ninguno se inicia sin haber sido seleccionado en las preferencias.

### Dónde guarda sus cosas

| | Preferencias | Descompilaciones en caché |
| --- | --- | --- |
| Linux | `$XDG_DATA_HOME/desdec/app.ron` o `~/.local/share/desdec/app.ron` | `$XDG_CACHE_HOME/desdec/decompiled` o `~/.cache/desdec/decompiled` |
| macOS | `~/Library/Application Support/Desdec/app.ron` | `~/Library/Caches/desdec/decompiled` |
| Windows | `%APPDATA%\Desdec\data\app.ron` | `%LOCALAPPDATA%\desdec\decompiled` |

Las preferencias se escriben una fracción de segundo después de dejar de
cambiar, y se vuelcan al disco en ese momento —sin esperar a un guardado
periódico ni a un cierre limpio. Una ventana cerrada de golpe en Windows perdía
el tema elegido instantes antes; ya no es así. La persistencia puede
desactivarse por completo, lo que también borra lo ya guardado. Las
descompilaciones se almacenan en caché bajo el SHA-256 del archivo del que
provienen: un archivo truncado, que no tiene una huella fiable, nunca se
almacena.

## Desarrollo

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```

La batería de pruebas tarda unos veinte segundos y no exige nada instalado.
Analiza binarios ELF, PE y Mach-O AArch64 sintéticos, forjados byte a byte en
`desdec-core::fixtures`: así, los lectores de los formatos que la máquina
anfitriona no usa se ejercitan en cada ejecución, en todas las plataformas.

Para revisar el juego de iconos tras modificar un glifo:

```sh
DESDEC_ICON_SHEET=/tmp/icons.svg cargo test -p desdec-app icon_sheet
```

### Organización

- `crates/desdec-core` — inspección y análisis de binarios. No sabe nada de
  ninguna interfaz. La lectura de entradas no confiables es acotada y total:
  cada lectura pasa por accesores comprobados, cada recorrido de tabla tiene
  tope, y ninguna entrada puede provocar un pánico.
- `crates/desdec-app` — la aplicación nativa `egui`.
- `docs/ARCHITECTURE.md` — el sentido de las dependencias y lo que queda
  deliberadamente fuera del núcleo.
- `docs/ai-collaboration/WORKLOG.md` — las reglas de trabajo comunes a los
  colaboradores humanos y a los asistentes de IA.

## Licencia

Apache-2.0 O MIT, a su elección: [LICENSE-APACHE](LICENSE-APACHE) y
[LICENSE-MIT](LICENSE-MIT). Ambas son accesibles también desde la ventana
Acerca de, de modo que los términos se alcanzan desde la propia aplicación.

Salvo que usted indique lo contrario, cualquier contribución que envíe
deliberadamente para su inclusión en este trabajo tendrá licencia doble como
arriba, sin términos ni condiciones adicionales.
