# Canvas de terminais em GPUI — desenho

Data: 2026-11-21

## Intenção

Um app desktop nativo, em Rust, que abre um canvas escuro gradeado e permite colocar nele vários terminais — cada um rodando um agente de IA — para acompanhar todos ao mesmo tempo. A referência visual é a imagem que o usuário mandou: janelas de terminal espalhadas pelo canvas, com títulos, soltas umas das outras.

O pedido explícito foi começar pequeno: **ter um terminal e conseguir apagar e adicionar mais**. Todo o resto entra em camadas posteriores.

## O que o usuário confirmou

- O canvas é o espaço de trabalho; o terminal é a unidade de trabalho.
- Arrastar e redimensionar janelas importa — sem isso não é canvas.
- Linhas tracejadas e post-it são camada futura, **não** fazem parte do primeiro corte.
- "Apagar" **encerra o processo do agente**, não esconde a janela.
- O framework é o [auro](https://github.com/pdl-clay/auro); a fonte a consultar é o repositório e a documentação publicada, não um checkout local.

## Goals

1. Abrir uma janela com um canvas gradeado em GPUI, sem webview.
2. Criar um terminal no canvas por ação do usuário; criar quantos quiser.
3. Apagar um terminal, encerrando o processo associado.
4. Selecionar, arrastar e redimensionar janelas de terminal.
5. Cada terminal ser um PTY real, com cores, teclado e redimensionamento de grade; scrollback dentro do que `gpui-terminal` entregar (ver decisão 13).

## Non-goals (primeiro corte)

- Linhas/conectores entre janelas.
- Post-it ou qualquer outro tipo de nó que não seja terminal.
- Persistência do layout entre execuções.
- Múltiplas janelas nativas, plugins, capabilities, updater, bundle.
- Seletor de comando por terminal, status de agente, agrupamento, zoom/pan.

## Anti-goals (o que tornaria isso uma falha mesmo funcionando)

- Virar um "Canva" completo com edição de texto, formas e exportação.
- Usar webview, HTML, CSS ou JavaScript no runtime — isso contraria o motivo de usar auro.
- Mostrar um terminal de mentira (texto estático) no lugar de um PTY real.
- Apagar a janela e deixar o processo do agente rodando solto.

## Restrições descobertas na pesquisa

- auro é **pré-alpha**: `0.1.0-alpha.10` no crates.io; a API não é estável. Fixar versões exatas.
- O workspace do auro fixa `gpui = "=0.2.2"` ([Cargo.toml](https://raw.githubusercontent.com/pdl-clay/auro/main/Cargo.toml)). Qualquer dependência de UI precisa bater com essa versão.
- auro **não é** biblioteca de componentes: ele fornece a camada de aplicação; widgets e renderer vêm de GPUI diretamente ([README](https://github.com/pdl-clay/auro)).
- O scaffold gerado por `auro new` tem quatro crates: bin, `ui` (Render), `data` (fronteira de hot reload) e `types`.
- `App::new().root::<V>("label").run()` Liga a view ao rótulo declarado em `Auro.toml`; `App::setup` é o ponto para inicializar toolkit/tema.
- Alvo local é Linux (Wayland/X11); `auro doctor` verifica as bibliotecas nativas e o pin de GPUI.
- Rust edition 2024, `rust-version = 1.90`.

## Decisões tomadas

Marcadas com **\[seu]** quando vieram do usuário e **\[minha]** quando são escolhas minhas, feitas para o documento não travar.

1. **[seu]** Runtime: auro + GPUI, frontend em Rust, zero webview.
2. **[seu]** Escopo do primeiro corte: terminal, adicionar, apagar.
3. **[seu]** Apagar mata o processo; conectores e post-it ficam para depois; arrastar/redimensionar importam.
4. **[minha]** Motor de terminal: `gpui-terminal` 0.1.0 + `portable-pty` 0.9. É o único crate publicado cuja dependência é exatamente `gpui = "0.2.2"` ([Cargo.toml](https://raw.githubusercontent.com/zortax/gpui-terminal/main/Cargo.toml)); ele traz `TerminalView` (implementa `Render`), callbacks de resize/exit/title e aceita os streams de Reader/Writer do PTY. Alternativas descartadas: `bezel-terminal` usa um fork `bezel-gpui` 0.3.25, e `gpui-libghostty` depende de um rev do git do Zed — ambos saem do pin do auro.
5. **[minha]** O canvas é uma view GPUI própria, com grade pintada diretamente; não entram bibliotecas de canvas terceiras neste corte.
6. **[minha]** Cada terminal é uma entidade GPUI que guarda o ID, o `ChildProcess`/PTY e o `TerminalView`; o estado do canvas guarda a lista de IDs e as posições.
7. **[minha]** Comando inicial de cada terminal: `$SHELL` (com `TERM=xterm-256color`). Escolher qual CLI de agente rodar por terminal fica para a camada seguinte — o primeiro corte prova o ciclo de vida do PTY, não a integração com um agente específico.
8. **[minha]** Sem diálogo de confirmação para apagar: o × na barra do terminal e o botão "Apagar selecionado" agem na hora.
9. **[minha]** Arrastar e redimensionar entram no primeiro corte porque definem o canvas; snap, zoom e pan ficam para depois.
10. **[minha]** Sem persistência: fechar e reabrir o app começa com um canvas vazio.
11. **[minha]** Quando o PTY termina sozinho (`exit`, agente finalizado ou crash), o cartão permanece com a saída final e um marcador "encerrado"; o × continua removendo o cartão. Não há remoção automática — preservar a saída de um agente que terminou vale mais que manter o canvas limpo.
12. **[minha]** "Matar o processo" significa encerrar o grupo de processos no Unix, não só o filho direto, e ao fechar o app todos os PTYs vivos são encerrados. É a implementação fiel do anti-goal de não deixar processo de agente solto.
13. **[minha]** Scrollback parcial no primeiro corte é aceitável: se `gpui-terminal` não entregar scrollback completo, segue assim e o limite fica registrado como conhecido, sem biblioteca alternativa (as outras quebram o pin do GPUI).

## Forma do primeiro corte

Uma janela (`Auro.toml` com um rótulo `main`). O conteúdo é o canvas gradeado. Acima dele, uma barra de ferramentas com "Adicionar terminal" e "Apagar selecionado". Cada terminal aparece como um cartão: barra de título com nome, botão × e corpo preenchido pelo `TerminalView`. Clique no cartão seleciona; arrastar pela barra move; redimensionar pela borda altera as dimensões do cartão e dispara o callback de resize do PTY.

Ao adicionar: gerar ID, abrir PTY com as dimensões iniciais, criar o `TerminalView` com os streams, registrar no estado, posicionar o cartão. Ao apagar: encerrar o grupo de processos do PTY, soltar os handles, remover a entidade do estado e repintar. Se o processo terminar por conta própria, o cartão fica com a saída final e o marcador "encerrado"; o × remove. Ao fechar o app, todos os PTYs vivos são encerrados. O foco de teclado vai para o terminal clicado; o app intercepta apenas as teclas de atalho da barra.

## Camadas seguintes (fora deste desenho)

- Conectores tracejados e post-it.
- Comando configurável por terminal e detecção de status do agente.
- Persistência do layout.
- Zoom/pan, snap e organize automático.

## Riscos a verificar cedo

- `gpui-terminal` 0.1.0 tem uma única versão publicada (dez/2025); a tabela de features marca seleção com mouse e scrollback como parciais/planejados. Confirmar compilação e comportamento real antes de construir em cima.
- O pin `gpui = "=0.2.2"` do auro e o `alacritty_terminal` 0.25.1 do gpui-terminal precisam resolver juntos sem conflito de versão.
- auro é alpha: quebrar a API entre alphas é esperado; não atualizar sem revisar este documento.

## O que fica para o builder

A arquitetura concreta em Rust (crates, tipos, entidades), o desenho dos elementos GPUI, o wiring exato de eventos de ponteiro/teclado, o tratamento de erro, os testes e a ordem de implementação. Este documento fixa o **quê** e **por quê**, não a API exata de cada chamada.
