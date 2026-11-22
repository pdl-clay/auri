# Canvas de terminais em GPUI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Abrir uma janela Auro com um canvas gradeado onde o usuário adiciona e apaga terminais PTY reais, podendo arrastá-los e redimensioná-los.

**Architecture:** O scaffold de quatro crates do auro (`bin`, `ui`, `data`, `types`) é gerado em `canvas/`. O estado lógico (ids, seleção, encerrados) vive em `types`, sem dependência de framework; geometria e PTY vivem em `ui`, num `TerminalCard` por terminal; o `Canvas` renderiza a grade, a barra de ferramentas e os cartões. Cada cartão usa `gpui-terminal::TerminalView` ligado a um `portable-pty`, e `PtySession::drop` encerra o grupo de processos.

**Tech Stack:** Rust edition 2024 (rust-version 1.90), auro `0.1.0-alpha.10`, GPUI `=0.2.2` (transitivo via auro), `gpui-terminal 0.1.0`, `portable-pty 0.9`, `parking_lot 0.12`, `anyhow 1`, `libc 0.2` (Unix), `tempfile 3` (dev).

**Spec:** `docs/superpowers/specs/2026-11-21-canvas-terminals-design.md`

## Global Constraints

- auro `0.1.0-alpha.10`; não atualizar o alpha sem revisar o spec.
- `gpui = "=0.2.2"` é fixado pelo auro — **não** declarar `gpui` diretamente; `gpui-terminal 0.1.0` é o único crate publicado que bate com esse pin.
- Runtime sem webview, HTML, CSS ou JavaScript.
- App gerado em `canvas/` (a raiz não está vazia e `auro new` recusa diretório não vazio). Decisão do plano.
- Comando inicial: `$SHELL` com `TERM=xterm-256color`, `COLORTERM=truecolor`.
- Apagar encerra o **grupo** de processos no Unix; `Drop` de `PtySession` também encerra, cobrindo saída do app.
- Terminal que termina sozinho mantém o cartão com a saída final e marcador "encerrado".
- Sem persistência, sem diálogo de confirmação, sem conectores, post-it, zoom ou pan.

## Review Focus

1. Agente termina sozinho (`exit`/crash) — o cartão deve ficar com a saída visível e "encerrado", não desaparecer. Teste manual no Task 5, passo 5.
2. Processo descendente (shell → agente → workers) — apagar não pode deixar órfãos. Teste `terminate_kills_descendants` no Task 3, passo 1.
3. Fechar o app com terminais vivos não pode deixar processo solto. Verificação no Task 5, passo 6 (`Drop` + `ps`).
4. Redimensionar o cartão tem de sincronizar a grade do PTY. Teste `grid_size` no Task 4, passo 1 + checagem manual no passo 5.
5. Digitar num terminal não pode vazar para outro; foco segue o clique. Checagem manual no Task 5, passo 5.

---

### Task 1: Scaffold e versões fixadas

**Files:**
- Create: `canvas/` (workspace inteiro gerado por `auro new canvas`)
- Modify: `canvas/Auro.toml`, `canvas/ui/Cargo.toml`

**Interfaces:**
- Consumes: nada.
- Produces: workspace compilável com `gpui 0.2.2` resolvido; rótulo de janela `main` em `Auro.toml`.

- [ ] **Step 1: Instalar a CLI alpha e gerar o scaffold**

```bash
cargo install auro-cli --version 0.1.0-alpha.10
auro --version
cd /home/pdl/Documentos/auri && auro new canvas
```

Expected: `auro --version` imprime `0.1.0-alpha.10`; `canvas/` contém `Auro.toml`, `src/main.rs`, `ui/`, `data/`, `types/`.

- [ ] **Step 2: Declarar dependências em `canvas/ui/Cargo.toml`**

Adicionar ao `[dependencies]`: `anyhow = "1"`, `gpui-terminal = "0.1.0"`, `libc = "0.2"`, `parking_lot = "0.12"`, `portable-pty = "0.9"`. Ao `[dev-dependencies]`: `tempfile = "3"`. Não adicionar `gpui`.

- [ ] **Step 3: Ajustar `canvas/Auro.toml`**

Manter um único window com label `main` e título "Canvas". Não habilitar `[dev] hot`.

- [ ] **Step 4: Inicializar git em `canvas/` se ainda não houver `.git`**

```bash
cd canvas && test -d .git || git init
```

- [ ] **Step 5: Verificar compile e pin**

```bash
cargo check --manifest-path /home/pdl/Documentos/auri/canvas/Cargo.toml
cargo tree --manifest-path /home/pdl/Documentos/auri/canvas/Cargo.toml -p gpui --depth 0
```

Expected: `check` passa; `gpui v0.2.2` aparece na árvore.

- [ ] **Step 6: Commit**

```bash
cd /home/pdl/Documentos/auri/canvas && git add -A && git commit -m "chore: scaffold auro canvas app with pinned terminal deps"
```

---

### Task 2: Estado lógico do canvas

**Files:**
- Create: `canvas/types/src/lib.rs` (ou o arquivo de biblioteca existente do crate `types`)

**Interfaces:**
- Consumes: nada.
- Produces: `pub type TerminalId = u64`; `pub struct TerminalState { pub id: TerminalId, pub title: String, pub exited: bool }`; `pub struct CanvasState` com `new()`, `add() -> TerminalState`, `remove(id) -> Option<TerminalState>`, `select(id)`, `selected() -> Option<TerminalId>`, `mark_exited(id)`, `terminals() -> &[TerminalState]`. `add` gera ids sequenciais a partir de 1 e título `Terminal {id}`; `add` também seleciona o novo terminal; `remove` limpa a seleção se o removido era o selecionado.

- [ ] **Step 1: Escrever testes que falham em `canvas/types/src/lib.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_generates_sequential_ids_and_selects() {
        let mut state = CanvasState::new();
        let a = state.add();
        let b = state.add();
        assert_eq!((a.id, b.id), (1, 2));
        assert_eq!((a.title.as_str(), b.title.as_str()), ("Terminal 1", "Terminal 2"));
        assert_eq!(state.selected(), Some(2));
    }

    #[test]
    fn remove_clears_selection_only_when_removing_selected() {
        let mut state = CanvasState::new();
        let a = state.add();
        let _b = state.add();
        state.select(a.id);
        state.remove(a.id);
        assert_eq!(state.selected(), None);
        assert_eq!(state.terminals().len(), 1);
    }

    #[test]
    fn mark_exited_preserves_terminal_and_output_flag() {
        let mut state = CanvasState::new();
        let a = state.add();
        state.mark_exited(a.id);
        assert!(state.terminals()[0].exited);
        assert_eq!(state.terminals().len(), 1);
    }
}
```

- [ ] **Step 2: Rodar e confirmar que falha**

Run: `cargo test --manifest-path /home/pdl/Documentos/auri/canvas/types/Cargo.toml`
Expected: FAIL — `CanvasState` não existe.

- [ ] **Step 3: Implementar `TerminalId`, `TerminalState` e `CanvasState` com as assinaturas acima**

`terminals` é `Vec<TerminalState>`, `selected` é `Option<TerminalId>`, `next_id` começa em 1. `mark_exited` é idempotente.

- [ ] **Step 4: Rodar e confirmar que passa**

Run: `cargo test --manifest-path /home/pdl/Documentos/auri/canvas/types/Cargo.toml`
Expected: 3 testes PASS.

- [ ] **Step 5: Commit**

```bash
cd /home/pdl/Documentos/auri/canvas && git add types && git commit -m "feat: logical canvas state"
```

---

### Task 3: Sessão PTY com encerramento de grupo

**Files:**
- Create: `canvas/ui/src/pty.rs`
- Modify: `canvas/ui/src/lib.rs` (declarar `pub mod pty;`)

**Interfaces:**
- Consumes: nada de tasks anteriores.
- Produces: `pub struct PtySession` com `pub reader: Box<dyn std::io::Read + Send>`, `pub writer: Box<dyn std::io::Write + Send>`, `pub master: Arc<parking_lot::Mutex<Box<dyn portable_pty::MasterPty + Send>>>`, `pub fn spawn_shell(cols: u16, rows: u16) -> anyhow::Result<Self>`, `pub fn spawn_command(cmd: portable_pty::CommandBuilder, cols: u16, rows: u16) -> anyhow::Result<Self>`, `pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()>`, `pub fn terminate(&mut self) -> anyhow::Result<()>`, `pub fn pid(&self) -> Option<u32>`, e `impl Drop for PtySession` que chama `terminate()` ignorando erro.

- [ ] **Step 1: Escrever testes que falham em `canvas/ui/src/pty.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use portable_pty::Child;

    #[test]
    fn spawn_resize_and_terminate() {
        let mut pty = PtySession::spawn_shell(80, 24).unwrap();
        assert!(pty.pid().is_some());
        pty.resize(100, 30).unwrap();
        pty.terminate().unwrap();
        assert!(pty.child.try_wait().unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn terminate_kills_descendants() {
        let marker = tempfile::NamedTempFile::new().unwrap();
        let mut script = CommandBuilder::new("/bin/sh");
        script.arg("-c");
        script.arg(format!("sleep 30 & echo $! > {}; wait", marker.path().display()));
        let mut pty = PtySession::spawn_command(script, 80, 24).unwrap();
        // esperar o arquivo aparecer, ler o pid, pty.terminate(), então
        // afirmar que libc::kill(bg_pid, 0) falha com ESRCH em até 2 s.
        let bg = read_pid_with_timeout(marker.path(), std::time::Duration::from_secs(2)).unwrap();
        pty.terminate().unwrap();
        assert!(wait_until_gone(bg, std::time::Duration::from_secs(2)));
    }
}
```

`read_pid_with_timeout` e `wait_until_gone` são helpers privados do mesmo módulo de teste; `child` é campo privado acessível pelo módulo filho.

- [ ] **Step 2: Rodar e confirmar que falha**

Run: `cargo test --manifest-path /home/pdl/Documentos/auri/canvas/ui/Cargo.toml pty::tests`
Expected: FAIL — `PtySession` não existe.

- [ ] **Step 3: Implementar `PtySession`**

`spawn_command` abre o PTY, roda o `CommandBuilder` recebido e devolve a sessão; `spawn_shell` é `spawn_command` com `CommandBuilder::new(shell)`, `shell = std::env::var("SHELL").unwrap_or("/bin/sh".into())`, env `TERM=xterm-256color` e `COLORTERM=truecolor`. Guardar `child` e `child.process_id()`. `resize` chama `MasterPty::resize`. `terminate`: no Unix, `libc::kill(-(pid as i32), libc::SIGTERM)` quando `pid` existe (o forkpty do portable-pty torna o filho líder de sessão/grupo), seguido de `child.kill()` como fallback; em outras plataformas, só `child.kill()`. `Drop` chama `terminate()` e descarta o resultado.

- [ ] **Step 4: Rodar e confirmar que passa**

Run: `cargo test --manifest-path /home/pdl/Documentos/auri/canvas/ui/Cargo.toml pty::tests`
Expected: 2 testes PASS.

- [ ] **Step 5: Commit**

```bash
cd /home/pdl/Documentos/auri/canvas && git add ui/src/pty.rs ui/src/lib.rs && git commit -m "feat: PTY session with process-group termination"
```

---

### Task 4: Cartão de terminal

**Files:**
- Create: `canvas/ui/src/card.rs`
- Modify: `canvas/ui/src/lib.rs` (declarar `pub mod card;`)

**Interfaces:**
- Consumes: `PtySession` (Task 3); `Canvas::mark_exited(id, cx)` e `Canvas::remove(id)` (Task 5).
- Produces: `pub struct TerminalCard` com `pub id: TerminalId`, `pub bounds: gpui::Bounds<gpui::Pixels>`, `pub exited: bool`, `pub terminal: Entity<TerminalView>`; `pub fn grid_size(width_px: f32, height_px: f32, font_px: f32, line_height: f32) -> (u16, u16)`; `pub fn new(id: TerminalId, bounds: Bounds<Pixels>, canvas: WeakEntity<Canvas>, cx: &mut Context<Self>) -> anyhow::Result<Self>`; `pub fn set_exited(&mut self, cx: &mut Context<Self>)`; `impl Render for TerminalCard`.

- [ ] **Step 1: Escrever teste que falha para `grid_size` em `canvas/ui/src/card.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::grid_size;

    #[test]
    fn grid_size_uses_monospace_metrics() {
        assert_eq!(grid_size(480.0, 300.0, 12.0, 1.2), (66, 20));
    }
}
```

Largura de caractere = `0.6 * font_px`; altura de linha = `font_px * line_height`; ambos com `floor`.

- [ ] **Step 2: Rodar e confirmar que falha**

Run: `cargo test --manifest-path /home/pdl/Documentos/auri/canvas/ui/Cargo.toml card::tests`
Expected: FAIL — `grid_size` não existe.

- [ ] **Step 3: Implementar `grid_size` e `TerminalCard`**

`TerminalCard::new`: derivar `(cols, rows)` de `bounds.size`, chamar `PtySession::spawn_shell(cols, rows)`, criar `TerminalView::new(writer, reader, config, cx)` dentro de `cx.new(...)` com `TerminalConfig` usando `font_size: px(12.)`, `line_height_multiplier: 1.2` e paleta padrão; callback de resize chama `pty.master.lock().resize(...)`; callback de exit chama `canvas.update(cx, |c, cx| c.mark_exited(id, cx))`. `Render`: barra de título com `Terminal {id}`, selo "encerrado" quando `exited`, botão × que chama `Canvas::remove`, e o `TerminalView` como filho. Pointer down na barra arrasta o cartão atualizando `bounds`; pointer down na borda redimensiona e chama `terminal.update(...).resize(...)` com a nova grade.

- [ ] **Step 4: Rodar teste e checar compilação**

```bash
cargo test --manifest-path /home/pdl/Documentos/auri/canvas/ui/Cargo.toml card::tests
cargo check --manifest-path /home/pdl/Documentos/auri/canvas/Cargo.toml
```
Expected: 1 teste PASS; check sem erros.

- [ ] **Step 5: Checagem visual**

Run: `auro dev --dir /home/pdl/Documentos/auri/canvas`
Observar: cartão aparece, `echo oi` aparece no terminal, arrastar pela barra move, arrastar borda redimensiona sem distorcer o texto.

- [ ] **Step 6: Commit**

```bash
cd /home/pdl/Documentos/auri/canvas && git add ui/src/card.rs ui/src/lib.rs && git commit -m "feat: terminal card with drag, resize and exit marker"
```

---

### Task 5: Canvas, barra de ferramentas e ciclo de vida

**Files:**
- Create: `canvas/ui/src/canvas.rs`
- Modify: `canvas/ui/src/lib.rs` (declarar `pub mod canvas;` e expor a view raiz), `canvas/src/main.rs` se necessário para ligar a root ao label `main`.

**Interfaces:**
- Consumes: `CanvasState` (Task 2), `TerminalCard` (Task 4).
- Produces: `pub struct Canvas` com `pub fn add_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>)`, `pub fn remove(&mut self, id: TerminalId)`, `pub fn remove_selected(&mut self)`, `pub fn select(&mut self, id: TerminalId, cx: &mut Context<Self>)`, `pub fn mark_exited(&mut self, id: TerminalId, cx: &mut Context<Self>)`; `impl Render for Canvas`.

- [ ] **Step 1: Escrever teste que falha para remoção via canvas em `canvas/ui/src/canvas.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_selected_drops_state_entry() {
        let mut canvas = Canvas::new_for_test();
        canvas.state.add();
        canvas.remove_selected();
        assert!(canvas.state.terminals().is_empty());
    }
}
```

`new_for_test` é `#[cfg(test)]`, constrói `Canvas` com `CanvasState::new()` e nenhum card. O teste não cria janela nem PTY.

- [ ] **Step 2: Rodar e confirmar que falha**

Run: `cargo test --manifest-path /home/pdl/Documentos/auri/canvas/ui/Cargo.toml canvas::tests`
Expected: FAIL — `Canvas` não existe.

- [ ] **Step 3: Implementar `Canvas`**

`Render`: fundo com grade desenhada em `canvas`, barra no topo com "Adicionar terminal" e "Apagar selecionado", cartões posicionados absolutamente em `bounds`, clique no cartão chama `select`. `add_terminal`: `state.add()` → `cx.new(|cx| TerminalCard::new(id, bounds, cx.entity().downgrade(), cx))` → guardar entidade → focar o `TerminalView`. `remove`: `state.remove(id)` → dropar a entidade do cartão (o `Drop` de `PtySession` encerra o grupo). `mark_exited`: `state.mark_exited(id)` e `card.update(cx, |c, cx| c.set_exited(cx))`.

- [ ] **Step 4: Rodar teste e checar compilação**

```bash
cargo test --manifest-path /home/pdl/Documentos/auri/canvas/ui/Cargo.toml canvas::tests
cargo check --manifest-path /home/pdl/Documentos/auri/canvas/Cargo.toml
```
Expected: 1 teste PASS; check sem erros.

- [ ] **Step 5: Checagem ponta a ponta (cobre Review Focus 1 e 5)**

Run: `auro dev --dir /home/pdl/Documentos/auri/canvas`
Fazer: adicionar dois terminais; digitar `echo um` no primeiro e `echo dois` no segundo — cada saída aparece só no seu cartão; digitar `exit` no primeiro — o cartão permanece com a saída e o selo "encerrado"; clicar × nesse cartão — ele desaparece.

- [ ] **Step 6: Checagem de shutdown (Review Focus 3)**

Com dois terminais vivos, anotar os pids (`ps -ef | grep -E 'bash|sh'`), fechar a janela do app e confirmar que os processos e seus filhos não sobrevivem.

- [ ] **Step 7: Commit**

```bash
cd /home/pdl/Documentos/auri/canvas && git add ui/src/canvas.rs ui/src/lib.rs && git commit -m "feat: canvas grid, toolbar and terminal lifecycle"
```

---

## Self-review

- Spec coverage: goals 1–5 → Tasks 1, 5, 5, 4, 3/4; non-goals respeitados; anti-goals cobertos por `Drop`/grupo de processos (Task 3) e pela checagem de shutdown (Task 5).
- Camadas futuras do spec não aparecem em nenhuma task, como pedido.
- Tipos consistentes: `TerminalId`/`TerminalState` de `types` são usados por `card.rs` e `canvas.rs`; `PtySession` de `pty.rs` é usado por `card.rs`; `Canvas` expõe exatamente `mark_exited` e `remove` que `card.rs` consome.
