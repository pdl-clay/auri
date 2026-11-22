# Drawer de espaços de trabalho — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Adicionar um drawer lateral de espaços de trabalho com criar/apagar, ícone, nome, raiz escolhida por seletor nativo, filtro, persistência em JSON e troca de canvas sem encerrar agentes.

**Architecture:** `canvas-types` ganha os tipos puros de workspace e a persistência; `canvas-ui` ganha um módulo `workspaces` com o drawer, o formulário e o config, e `Canvas` passa a guardar um `CanvasState` por espaço. Cada espaço mantém seus cartões vivos em memória; apagar um espaço dropar as entidades dele, disparando o `PtySession::Drop` existente. A raiz é escolhida com `cx.prompt_for_paths` do GPUI 0.2.2 (já reexportado pelo auro), com fallback de campo de texto em `Err`.

**Tech Stack:** Rust 2024, auro `=0.1.0-alpha.10`, GPUI `=0.2.2` via `auro::gpui`, `gpui-component = "=0.5.1"` (Input/InputState), `gpui-terminal 0.1.0`, `portable-pty 0.9`, `serde`/`serde_json`, `tempfile` (dev).

**Spec:** `docs/superpowers/specs/2026-11-22-workspaces-drawer-design.md`

## Global Constraints

- Manter `auro = "=0.1.0-alpha.10"` e não declarar `gpui` diretamente. `gpui-component = "=0.5.1"` é declarado diretamente no manifesto do app, sem wrapper `auro-*`.
- Nenhum crate novo de diálogo: o seletor é `cx.prompt_for_paths` com `PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(...) }`; retorno `oneshot::Receiver<Result<Option<Vec<PathBuf>>>>`; `None` = cancelar, `Err` = fallback de texto.
- `PathPromptOptions` não aceita diretório inicial; não fingir que aceita.
- Runtime sem webview, HTML, CSS ou JavaScript.
- Sem renomear/editar espaço, sem reordenar, sem diálogo de confirmação para apagar.
- Config em `XDG_CONFIG_HOME` (ou `~/.config`)/`auro-canvas/workspaces.json`; arquivo corrompido é preservado, nunca apagado em silêncio.
- Paleta de ícones fixa: estrela, pasta, raio, código, globo. Usar `auro::gpui::svg()` com cinco arquivos `.svg` empacotados em `canvas/icons/`; sem crate de ícones e sem emoji.
- Tipos de workspace ficam em `canvas-types`, sem GPUI nem auro.

## Review Focus

1. Raiz inexistente/inacessível após load — espaço permanece marcado inválido, erro aparece ao criar terminal; teste no Task 1.
2. JSON corrompido — bytes preservados, salvamento tolerante, "Recomeçar" faz backup antes de reescrever; teste no Task 1.
3. Trocar de espaço não pode encerrar terminais; apagar espaço encerra só os dele; teste no Task 4 + checagem com `ps`.
4. Cancelar o seletor não cria espaço nem erro; teste/verificação no Task 3.
5. Digitar no filtro não pode vazar para o formulário nem para o terminal; checagem visual no Task 4.

---

### Task 1: Tipos de workspace e persistência

**Files:**
- Modify: `canvas/types/Cargo.toml`, `canvas/types/src/lib.rs`
- Create: `canvas/ui/src/config.rs`
- Modify: `canvas/ui/Cargo.toml`, `canvas/ui/src/lib.rs`

**Interfaces:**
- Consumes: nada.
- Produces: em `canvas-types`: `pub type WorkspaceId = u64`; `pub enum WorkspaceIcon { Star, Folder, Bolt, Code, Globe }`; `pub struct WorkspaceState { pub id: WorkspaceId, pub name: String, pub icon: WorkspaceIcon, pub root: PathBuf, pub invalid_root: bool, pub canvas: CanvasState }`; `pub struct WorkspaceStore { workspaces: Vec<WorkspaceState>, active: Option<WorkspaceId>, next_id: WorkspaceId }` com `new()`, `add(name, icon, root) -> WorkspaceState`, `remove(id) -> Option<WorkspaceState>`, `select(id)`, `active() -> Option<WorkspaceId>`, `active_mut() -> Option<&mut WorkspaceState>`, `workspaces() -> &[WorkspaceState]`.
- `ui::config`: `pub fn config_path() -> PathBuf`, `pub fn load(store: &mut WorkspaceStore) -> LoadOutcome`, `pub fn save(store: &WorkspaceStore) -> anyhow::Result<()>`. `LoadOutcome` enum: `Fresh`, `Loaded`, `Recovered`, `Corrupt` (com o erro em `String`). `Recovered`/`Corrupt` preservam o arquivo e sinalizam recuperação na UI.

- [ ] **Step 1: TDD — escrever testes que falham**

Em `canvas/types/src/lib.rs`: ids sequenciais, `active` após `add`, `remove` limpando `active`, `invalid_root` serializável. Em `canvas/ui/src/config.rs`: `load` com arquivo malformado retorna `Corrupt`/`Recovered` e **não** apaga nem sobrescreve o arquivo; `save` + `load` preservam nome/ícone/raiz; `config_path` usa `XDG_CONFIG_HOME` quando definido.

- [ ] **Step 2: Rodar e confirmar RED**

Run: `cargo test --workspace --manifest-path /home/pdl/Documentos/auri/.worktrees/canvas-terminals/canvas/Cargo.toml`
Expected: falha de compilação pelos tipos ausentes.

- [ ] **Step 3: Implementar tipos e config**

`serde::{Serialize, Deserialize}` com `#[serde(default)]` em campos novos; `serde_json` para leitura/escrita. `config_path` usa `std::env::var_os("XDG_CONFIG_HOME")` ou `~/.config`, sem crate novo. Recuperação: ao falhar o parse, tentar extrair entradas completas; se conseguir, carregar e retornar `Recovered`; se não, retornar `Corrupt` e seguir com o espaço inicial em memória sem escrever. `save` nunca sobrescreve um arquivo corrompido até o chamador pedir "Recomeçar" (que copia para `.bad-<timestamp>` antes).

- [ ] **Step 4: Rodar e confirmar GREEN**

Run: `cargo test --workspace --manifest-path .../canvas/Cargo.toml`
Expected: todos passam, sem warnings novos além do `proc-macro-error2` transitivo.

- [ ] **Step 5: Commit**

`git commit -m "feat: workspace types and persisted config"`

---

### Task 2: PTY com diretório de trabalho

**Files:**
- Modify: `canvas/ui/src/pty.rs`

**Interfaces:**
- Consumes: nada.
- Produces: `pub fn spawn_shell_in(cols: u16, rows: u16, cwd: Option<&std::path::Path>) -> anyhow::Result<Self>`; `spawn_shell(cols, rows)` passa a delegar com `cwd = None`.

- [ ] **Step 1: TDD — teste em `pty.rs`**

`spawn_shell_in` com um diretório temporário: o comando `pwd` escrito no `writer` tem que devolver exatamente aquele caminho lido do `reader` (com `\r\n` normalizado). Teste também que `cwd = None` mantém o comportamento atual.

- [ ] **Step 2: RED, implementar com `CommandBuilder::cwd(cwd)`, GREEN**

Run: `cargo test --manifest-path .../canvas/ui/Cargo.toml pty::tests`
Expected: novo teste falha antes, passa depois; testes de group-kill continuam verdes.

- [ ] **Step 3: Commit**

`git commit -m "feat: spawn terminal shell in workspace root"`

---

### Task 3: Drawer, filtro e formulário com seletor nativo

**Files:**
- Create: `canvas/ui/src/workspaces.rs`, `canvas/icons/star.svg`, `canvas/icons/folder.svg`, `canvas/icons/bolt.svg`, `canvas/icons/code.svg`, `canvas/icons/globe.svg`
- Modify: `canvas/ui/Cargo.toml`, `canvas/ui/src/lib.rs`

**Interfaces:**
- Consumes: `WorkspaceStore`/`WorkspaceState`/`WorkspaceIcon` (Task 1), `config::LoadOutcome` (Task 1).
- Produces: `pub enum DrawerAction { Create { name: String, icon: WorkspaceIcon, root: PathBuf }, Delete(WorkspaceId), Select(WorkspaceId) }` e `pub struct WorkspaceDrawer` com `pub fn new(load: LoadOutcome) -> Self`, `pub fn on_action(&mut self, handler: impl Fn(DrawerAction) + 'static)`, `pub fn filter(&self) -> &str`, `pub fn set_filter(&mut self, value: String)`, `pub fn open_form(&mut self)`, `pub fn close_form(&mut self)`, `pub fn set_form_name(&mut self, value: String)`, `pub fn set_form_icon(&mut self, icon: WorkspaceIcon)`, `pub fn set_form_root(&mut self, root: Option<PathBuf>)`, `pub fn set_form_error(&mut self, error: Option<String>)`, `pub fn is_form_open(&self) -> bool`; `impl Render for WorkspaceDrawer`. O `Canvas` consome `DrawerAction` por esse callback — sem referência circular.

- [ ] **Step 1: TDD — teste de filtro puro em `workspaces.rs`**

`filter_matches(name, query)` com query vazia → `true`; case-insensitive; substring; acentos não precisam ser normalizados.

- [ ] **Step 2: RED → integrar `gpui-component` → GREEN**

Adicionar `gpui-component = "=0.5.1"` a `canvas/ui/Cargo.toml`, chamar `gpui_component::init(cx)` no setup do `run()` em `ui/src/lib.rs` e usar `gpui_component::input::{Input, InputState}` para o campo de filtro e o nome do formulário (com o tema/Root do componente, conforme o testbed `native-gui`). Líder da framework confirmou: não existe input pronto no GPUI 0.2.2 nem no `auro-ui`. Filtro e nome não usam key handling artesanal.

- [ ] **Step 3: Implementar desenho do drawer**

Coluna fixa de ~200 px; campo "Filtrar" com `Input`; botão "+"; rótulo "ESPAÇOS DE TRABALHO"; lista com ícone SVG, nome, monitor e contagem de terminais; linha ativa realçada. Ícones: `svg().path("icons/<nome>.svg").w(px(16.)).h(px(16.))` a partir de `auro::gpui::svg`. Formulário inline com nome (`Input`), cinco ícones SVG selecionáveis, botão de escolher raiz e texto de erro.

- [ ] **Step 4: Ligar `cx.prompt_for_paths`**

No botão de raiz, chamar `cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Escolha a raiz".into()) })`; `None` fecha sem efeito; `Some(paths)` preenche o formulário; `Err` mantém o campo de raiz do formulário editável como fallback. Não inventar diretório inicial.

- [ ] **Step 5: Commit**

`git commit -m "feat: workspace drawer, filter and picker form"`

---

### Task 4: Integração no canvas e checagem ponta a ponta

**Files:**
- Modify: `canvas/ui/src/canvas.rs`, `canvas/ui/src/lib.rs`

**Interfaces:**
- Consumes: `WorkspaceStore` (Task 1), `WorkspaceDrawer` (Task 3), `PtySession::spawn_shell_in` (Task 2).
- Produces: `Canvas` passa a ter `store: WorkspaceStore`, `drawer: WorkspaceDrawer` e cartões por espaço; `add_terminal` usa a raiz do espaço ativo; `remove_workspace(id)` dropar os cartões e salvar config.

- [ ] **Step 1: TDD — teste de estado em `canvas.rs`**

`remove_workspace` remove apenas os cartões daquele espaço e limpa `active` quando era o ativo; criar espaço com raiz vazia é recusado; sem espaço ativo, `add_terminal` não cria nada.

- [ ] **Step 2: RED → integrar → GREEN**

Cartões agrupados por `WorkspaceId`; `Render` desenha o drawer à esquerda e, à direita, só os cartões do espaço ativo; trocar de espaço apenas esconde; toolbar "Adicionar terminal" desabilitada sem espaço ativo; contagem por espaço vem do `CanvasState` dele. **Ruling do controller, com evidência do líder da framework:** auro alpha.10 não expõe AssetSource — converter os cinco SVGs para PNG (`rsvg-convert`) e usar `img(ImageSource::Image(Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!(...).to_vec()))))` em vez de `svg().path(...)`; manter o pin `=0.1.0-alpha.10`. Após cada mutação do store, chamar `drawer.set_store(...)`; o `LoadOutcome` recebido é autoritativo para o banner.

- [ ] **Step 3: Apagar espaço**

Dropar as entidades de cartões dele, remover do `store`, chamar `config::save`, ativar o primeiro restante ou nenhum. Confirmar com `ps` que os processos do espaço apagado morreram e os dos outros continuam.

- [ ] **Step 4: Verificação visual real**

`auro dev --dir .../canvas`: criar dois espaços com raízes diferentes, chooser nativo, adicionar terminal em cada um (`pwd` confirma a raiz), filtrar, trocar de espaço sem perder saída, apagar um e verificar `ps`, reiniciar o app e ver a lista persistida.

- [ ] **Step 5: Commit**

`git commit -m "feat: wire workspaces into the terminal canvas"`

---

## Self-review

- Spec coverage: goals 1-6 → Tasks 1,3,4; decisões 8-17 do spec cobertas; anti-goals verificados no Review Focus.
- Tipos consistentes: `WorkspaceId`/`WorkspaceState`/`WorkspaceStore` de `types` alimentam `config` e `workspaces`; `LoadOutcome` de `config` alimenta o drawer; `spawn_shell_in` alimenta o `Canvas`.
- Fora de escopo: editar/reordenar espaços, conectarores/post-it, restauração automática de terminais — não aparecem em nenhuma task.
