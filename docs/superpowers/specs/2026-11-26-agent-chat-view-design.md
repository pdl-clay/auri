# Auri — visualização de chat por cartão de agente

Data: 2026-11-26
Status: desenho para aprovação

## Intenção

Cada cartão de agente passa a ter duas apresentações do mesmo processo:

1. **Terminal** — a visão atual, com o PTY e o renderer de terminal.
2. **Chat** — uma conversa em GPUI, com mensagens do usuário à direita,
   respostas finais do agente à esquerda, status vivo no meio e input no rodapé.

O terminal não muda de comportamento. A troca é só de apresentação; o mesmo
PTY, o mesmo agente e o mesmo cartão continuam por baixo.

O objetivo principal é leitura. O OMP às vezes é verbal demais no terminal;
o chat deve esconder ruído de ferramenta e entregar uma conversa limpa, com
markdown e blocos de código fáceis de copiar.

## Decisões tomadas com o usuário

- O botão de troca aparece em **qualquer cartão de agente** já aberto, não só
  no OMP.
- A troca fica no canto superior direito, como uma pequena overlay sobre o
  corpo do cartão. O ícone mostra o destino: bolha de conversa quando está no
  terminal, ícone de terminal quando está no chat.
- O cartão **lembra a última visualização usada** e reabre nela.
- No OMP, o chat abre com o **histórico da sessão já reconstruído**.
- Nos outros agentes, o chat abre com uma **mensagem informativa** explicando
  que o histórico não é estruturado e convidando a mandar a primeira mensagem.
- Só entram como bolhas: a mensagem do usuário e a resposta final do agente.
  Pensamento, chamadas de ferramenta e resultados viram uma **linha de status**:
  “pensando…”, “chamando ferramenta X”, “escrevendo…”.
- O input aceita várias mensagens enquanto o agente trabalha e **envia em fila**.
- Digitar `/` abre um seletor de skills. O OMP registra skill como
  `/skill:nome`; o seletor insere esse comando no input.
- Texto é renderizado como markdown. Blocos de código têm ação de **copiar**.

## Objetivos

- Trocar terminal ↔ chat sem recriar PTY, sem perder sessão e sem perder scroll
  do terminal.
- Reconstruir histórico e status do OMP a partir do JSONL de sessão.
- Manter o terminal 100% funcional; o chat é uma camada adicional.
- Ser simples no primeiro corte, mas com a estrutura pronta para evoluir.
- Respeitar os tokens visuais de `canvas/ui/src/theme.rs`.

## Não-objetivos do primeiro corte

- Não mudar o protocolo do OMP nem os CLIs dos outros agentes.
- Não editar, apagar ou reagir a mensagens.
- Não anexar arquivos, imagens ou áudio.
- Não mostrar cartões ricos de ferramenta; ferramenta é status.
- Não persistir o histórico de chat de agentes não-OMP.
- Não criar chat para terminais comuns, só para cartões de agente.
- Não fazer busca global, notificações ou múltiplas conversas por cartão.
- Não suportar tabelas complexas ou HTML arbitrário no primeiro corte; o
  `TextView` do `gpui-component` já cobre o markdown comum.

## Experiência

### Cabeçalho e troca

- O cartão continua com título, agente, controles de arrastar e fechar.
- Uma overlay circular/compacta no canto superior direito do corpo mostra o
  ícone da outra visualização e tem tooltip (“Ver como chat”, “Ver terminal”).
- O botão não interfere no arrastar do cartão, na seleção de conexão nem no
  botão de fechar.
- A overlay fica visível nos dois modos.

### Corpo no modo chat

- Lista vertical rolável de mensagens.
- Usuário: bolha à direita, alinhada ao fim da coluna.
- Agente: bolha à esquerda, com ícone/nome do agente no topo quando ajudar.
- Mensagem informativa do sistema: bloco discreto, largura total, cor muted.
- Status: uma linha viva abaixo da última mensagem, com indicador animado
  discreto e texto curto.
- Input no rodapé do corpo, acima da linha de caminho do cartão.
- O terminal continua montado por baixo da overlay de chat. A troca não
  desmonta o `TerminalView`.

### Input

- Enter envia; Shift+Enter quebra linha.
- Campo cresce até um limite e depois rola internamente.
- Enquanto o agente está ocupado, a mensagem entra numa fila FIFO e aparece
  como bolha do usuário com marca “na fila”.
- Quando o turno termina, a próxima mensagem da fila é enviada ao PTY.
- Botão de enviar desabilitado apenas quando o campo está vazio ou o agente
  encerrou.
- O input não envia texto para agentes encerrados.

**Submit por agente (decisão):** o OMP liga `ctrl+j`/`\n` a newline; enviar
`texto\n` não submete. O adaptador do OMP manda uma linha simples como
`texto\r`; texto com várias linhas vai em bracketed paste
(`ESC[200~…ESC[201~`) seguido de `\r`. Outros agentes usam `texto\r` no
primeiro corte. A diferença fica isolada num adaptador por `AgentKind`.

### Fila e fim de turno

- OMP: a fila libera quando o registro `message/assistant` tem `stopReason`
  diferente de `toolUse` (incluindo `aborted` e `error`). Sem esse sinal, a fila
  espera.
- Não-OMP: a fila libera quando o terminal fica quieto por ~900 ms depois de
  ter havido saída. É o melhor sinal disponível.
- Risco aceito no primeiro corte: se um agente não-OMP estiver parado num
  prompt interativo, a próxima mensagem pode cair nesse prompt. O status deve
  deixar claro que a resposta pode precisar do terminal.
- Se o agente encerrar, a fila é descartada e o input desabilita.

### Prompts interativos

- OMP `ask` e prompts de permissão bloqueiam o turno no terminal; no JSONL
  viram `tool_execution_start` com `toolName: "ask"` ou ficam sem evento final.
- Quando isso for detectado, o status vira **“responda no terminal”** e a
  overlay mostra um atalho para trocar para o terminal. A fila fica retida até
  o turno terminar.
- O chat não tenta responder formulários HTML nem prompts de permissão.

### Seletor de skills

- Abre quando o input começa com `/` (ou quando `/` é digitado no início de
  uma palavra, conforme o builder decidir).
- Lista skills com nome e descrição curta; filtra ao digitar.
- Selecionar insere `/skill:nome ` no input; não envia sozinho.
- Fontes de descoberta: os mesmos diretórios que o OMP usa para skills
  (`~/.agents/skills`, `.agents/skills` no workspace, `.omp/skills` e os demais
  diretórios já carregados pelo OMP). O primeiro corte pode limitar aos
  diretórios que o Auri instala/gerencia.
- Sem skills encontradas: estado vazio explícito.

### Markdown e blocos de código

- Usar `gpui_component::text::TextView::markdown`.
- `selectable(true)` para seleção de texto.
- `code_block_actions` para um botão “Copiar” no canto do bloco.
- Copiar escreve `code_block.code()` no clipboard do GPUI.
- Respostas de agentes não-OMP são texto de terminal: renderizar como texto
  pré-formatado, não como markdown, para não interpretar saída crua.

### Estados

- OMP com sessão encontrada: histórico carregado; status vivo.
- OMP sem sessão: informativo “não encontrei a sessão deste cartão; envie uma
  mensagem para começar” e input habilitado.
- OMP com erro de leitura: informativo de erro; input continua disponível.
- Não-OMP: informativo de histórico limitado; input habilitado.
- Agente encerrado ou foreground não verificado: status “use o terminal”;
  input desabilitado; histórico visível.
- Fila: bolhas pendentes marcadas “na fila”.
- Sem mensagens e sem status: apenas o informativo, nunca uma tela morta.

### Liveness do agente

Cada cartão roda um shell de login; o shell pode sobreviver ao agente. Antes de
enviar pelo chat, verificar se o processo em foreground do PTY ainda é um
agente. Se não for possível verificar, desabilitar o envio e mostrar
“use o terminal” em vez de arriscar executar a mensagem como comando de shell.

## Modelo de dados

### Persistência

Adicionar em `canvas/types/src/lib.rs`:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentView {
    #[default]
    Terminal,
    Chat,
}
```

`AgentState` ganha `#[serde(default)] pub view: AgentView`. Configs antigas
continuam abrindo no terminal. O canvas persiste a troca via o mesmo caminho
de `finish_store_change`.

### Estado de UI do chat

O `ChatView` mantém, em memória:

- `messages: Vec<ChatMessage>` — bolhas já confirmadas.
- `pending: VecDeque<PendingMessage>` — mensagens na fila.
- `status: Option<ChatStatus>`.
- `session: Option<PathBuf>` — caminho do JSONL do OMP.
- `transcript_offset: u64` e buffer de linha parcial para tail incremental.

`ChatMessage`:

```rust
enum ChatRole { User, Agent, Info }
struct ChatMessage {
    role: ChatRole,
    text: String,
    /// Markdown para OMP; texto pré-formatado para não-OMP.
    format: ChatFormat,
}
```

Nada disso é persistido no primeiro corte, exceto `AgentView`. No OMP o
histórico volta do JSONL; nos outros, o informativo volta a aparecer.

## OMP: leitura da sessão

### Formato observado

Sessões ficam em `~/.omp/agent/sessions/<cwd-slug>/<timestamp>_<id>.jsonl`.
Cada linha é um objeto. Os tipos relevantes:

- `message` com `message.role`:
  - `user`: conteúdo `text`.
  - `assistant`: conteúdo com blocos `thinking`, `text`, `toolCall`.
  - `toolResult`: resultado de ferramenta.
- `custom` com `customType`:
  - `tool_execution_start`, cujo `data` tem `toolName`, `toolCallId`, `intent`.
  - `session_exit`.
- `session`, `model_change`, `model_usage`, `title`, `title_change` e outros:
  ignorar para o chat.

### Regras de conversão

- `message`/`user` vira bolha do usuário.
- `message`/`assistant` **sem** nenhum bloco `toolCall` vira bolha final do
  agente, concatenando os blocos `text`. Blocos `thinking` não aparecem.
- `message`/`assistant` com `toolCall` não vira bolha; atualiza o status:
  - se tem `thinking`: “pensando…”;
  - senão: “chamando ferramenta <último toolName>”.
- `tool_execution_start`: “chamando ferramenta <toolName>”.
- `toolResult`: “processando resultado…” e, depois, “escrevendo…” até a próxima
  mensagem final ou ferramenta.
- `session_exit`: status “encerrado”.
- Linhas desconhecidas, inválidas ou parciais são ignoradas; a linha parcial
  fica no buffer até completar.
- Ao enviar uma mensagem do usuário, o status entra em “pensando…” até o
  primeiro evento do agente.

### Carga inicial e tail

- Ao abrir o chat, ler o arquivo do início em background.
- Renderizar no máximo as últimas 200 mensagens (ou ~2 MB de arquivo) no
  primeiro corte; o resto fica fora da tela. O builder pode ajustar o limite.
- Guardar o offset do fim do arquivo lido.
- Poll a cada ~150 ms no `background_executor`; ler somente bytes novos.
- Se o arquivo encolher ou for substituído, resetar offset e recarregar.
- Nunca bloquear a UI thread com I/O ou parsing.

### Descoberta do caminho da sessão

O cartão já recebe `agent.session` ao ser reconstruído e o
`track_omp_session` descobre o caminho pelo breadcrumb do TTY. O desenho
adiciona:

- `TerminalCard` guarda `session: Option<PathBuf>`.
- `spawn_cli_card`/`spawn_agent` passam o caminho salvo ao cartão.
- `track_omp_session`, quando descobre a sessão, atualiza o store **e** o
  cartão, para o chat começar a ler sem precisar recarregar o workspace.
- Se o cartão ainda não tem caminho, o chat mostra o informativo e passa a
  ler assim que o caminho chegar.

### Sessões compartilhadas

O fallback `omp --continue` pode fazer dois cartões apontarem para o mesmo
JSONL. Isso é aceito no primeiro corte: os dois chats leem o mesmo histórico e
mantêm filas independentes. Não há locking nem detecção de duplicidade; é uma
limitação conhecida, não um erro silencioso.

## Agentes não-OMP

- O chat abre com o informativo de histórico não estruturado.
- Enviar escreve no PTY e adiciona a bolha do usuário.
- A resposta do agente é uma única bolha à esquerda, alimentada por um snapshot
  do texto visível do terminal desde o último envio.
- O snapshot é atualizado quando o terminal fica quieto por um intervalo
  curto (ex.: 700–900 ms) após ter havido saída. Pode conter ruído de prompt;
  isso foi aceito.
- `visible_lines()` só vê o viewport; resposta maior que a tela fica truncada.
  Aceito no primeiro corte; não alterar o `gpui-terminal` vendored.
- Não há histórico entre aberturas; ao reabrir, o informativo volta.
- A bolha de resposta é texto pré-formatado, sem markdown.
- O builder deve isolar a heurística em uma função pura testável
  (`snapshot_response`) para não depender de timing nos testes.

## Arquitetura

Arquivos previstos:

- `canvas/types/src/lib.rs`: `AgentView`, campo em `AgentState`, testes de
  compatibilidade.
- `canvas/ui/src/chat.rs` (novo): parsing puro de JSONL, status, fila,
  deduplicação, snapshot de terminal, descoberta de skills, adaptador de envio.
- `canvas/ui/src/chat_view.rs` (novo): entidade GPUI do chat, lista, bolhas,
  input, seletor de skills e status.
- `canvas/ui/src/card.rs`: modo de visualização, botão de troca, overlay,
  sessão, render terminal/chat, foco.
- `canvas/ui/src/canvas.rs`: passar sessão ao cartão, atualizar cartão em
  `track_omp_session`, persistir a view.
- `canvas/ui/src/theme.rs`: tokens do chat.
- `canvas/ui/src/regressions.rs` e testes de unidade: parser, toggle, fila,
  informativos.
- `canvas/tests/visual/`: fixture e golden novo para o chat.

### Fluxo do toggle

1. Botão no cartão chama `Canvas::set_agent_view(workspace, id, view)`.
2. Canvas atualiza `AgentState.view`, persiste e notifica o cartão.
3. Cartão alterna a overlay local; terminal continua montado.
4. Ao reabrir o workspace, `spawn_cli_card` lê `AgentState.view` e o cartão
   começa no modo salvo.

### Fluxo de envio

1. Input chama `ChatView::send`.
2. Se o agente está ocupado, entra na fila e renderiza “na fila”.
3. Se está livre, o adaptador do agente escreve no `SharedPtyWriter` do cartão.
4. Bolha do usuário é confirmada quando o JSONL registra a mensagem
   correspondente (OMP) ou imediatamente (não-OMP e `/skill:`).
5. Ao final do turno, a fila avança.

### Deduplicação de mensagens do usuário

- OMP grava a mensagem do usuário no JSONL. Para não duplicar a bolha local,
  comparar texto normalizado e ordem com as pendências.
- Se a mensagem do JSONL não casar com nenhuma pendência, ela é histórica e
  entra normalmente.
- O builder deve manter essa regra em função pura, testável.
- `/skill:nome` não gera `message/user` no JSONL (gera `custom_message` /
  `skill-prompt`); por isso o envio de skill confirma a bolha imediatamente e o
  texto expandido da skill fica oculto.

### Observação de saída do terminal

O `gpui-terminal` não expõe evento de saída; o `Wakeup` do alacritty fica
interno. No primeiro corte, o chat faz polling/hash do texto visível a cada
~150 ms para detectar atividade e quietude. Não alterar o crate vendored neste
primeiro corte.

## Tokens visuais

Adicionar em `theme.rs` (nomes exatos ficam para o builder, mas todos os
valores visuais precisam morar lá):

- cor de fundo da bolha do usuário, do agente e do informativo;
- cor de borda/realce da bolha;
- raio e padding das bolhas;
- gap entre mensagens;
- largura máxima da bolha;
- tamanho do botão de troca e do botão copiar;
- altura mínima/máxima do input;
- fonte do status e do informativo;
- cor do indicador de status.

Nenhum hexadecimal ou número mágico de estilo dentro de `chat_view.rs`.

## Testes e verificação

- Parser OMP com fixtures JSONL:
  - usuário, resposta final, resposta intermediária com toolCall;
  - tool_execution_start, toolResult, session_exit;
  - linha inválida/truncada, tipo desconhecido;
  - limite de histórico e reset quando o arquivo encolhe.
- Fila:
  - várias mensagens enquanto ocupado;
  - uma enviada por vez;
  - ordem preservada;
  - mensagem confirmada não duplica;
  - `/skill:` confirma na hora.
- View:
  - toggle grava `AgentView` no store;
  - config antiga abre no terminal;
  - reabrir workspace respeita o modo salvo.
- Não-OMP:
  - informativo inicial;
  - snapshot vira bolha;
  - agente encerrado/foreground não verificado desabilita envio.
- Skills:
  - descoberta e filtro por nome;
  - inserção de `/skill:nome`.
- Visual:
  - golden do chat com bolhas, status, markdown e bloco de código;
  - golden do estado informativo sem sessão.

Rodar `cargo test --workspace` e `./scripts/visual-regression.sh --check`.

## Riscos e mitigação

- **Formato do JSONL do OMP é externo.** Parser tolerante, sem panic; tipos
  desconhecidos ignorados; testes com fixture.
- **Status não é streaming.** O JSONL grava mensagens completas; “escrevendo…”
  é uma inferência entre toolResult e mensagem final. Aceitável no primeiro
  corte.
- **Submit do OMP é sensível a newline.** Adaptador com `\r` e bracketed paste
  para múltiplas linhas; teste de bytes enviados.
- **Prompts interativos.** Status “responda no terminal” + atalho; fila retida.
- **Shell sobrevive ao agente.** Checar foreground antes de enviar; sem
  verificação, desabilitar o input.
- **Mensagens duplicadas.** Deduplicação por texto/ordem, com teste.
- **Sessões longas.** Limite de histórico e leitura incremental.
- **Snapshot de terminal ruidoso/truncado.** Aceito explicitamente; texto
  pré-formatado, sem markdown; apenas viewport.
- **Sessões compartilhadas.** Aceito; filas independentes, sem locking.
- **Foco/teclado.** O input do chat e o terminal não podem disputar foco;
  ao alternar, focar o input do chat; ao voltar, devolver foco ao terminal.
- **Overlay sobre terminal.** Manter o terminal montado; não recriar PTY.

## Fora do primeiro corte

- Anexos e imagens.
- Cartões de ferramenta expansíveis.
- Busca no histórico.
- Persistir histórico de não-OMP.
- Streaming token a token no chat.
- Múltiplas conversas por cartão.
- Notificações de mensagem.

## Chamadas minhas para o builder

- Usar `TextView::markdown` e `code_block_actions` do vendor em vez de escrever
  um renderer.
- Terminal sempre montado; chat é overlay.
- Fila é do Auri, não do agente: uma mensagem por vez, liberada quando o turno
  termina.
- OMP libera a fila por `stopReason != "toolUse"`; não-OMP por quietude do
  terminal.
- Histórico inicial limitado às últimas 200 mensagens; sem lazy-load no
  primeiro corte.
- Não-OMP com snapshot de terminal e texto pré-formatado.
- `/skill:nome` como sintaxe inserida; skill confirma bolha na hora.
- Envio do OMP com `\r` e bracketed paste para múltiplas linhas.
- Liveness do foreground antes de enviar; na dúvida, “use o terminal”.
