# Changelog

## [Não publicado]

### Corrigido

- O texto padrão do input do chat e o conteúdo do bloco de notas escalam com o
  zoom do canvas. O `Input` do vendor fixava a fonte em `input_text_size`; o
  composer e a nota agora definem `text_size` pela escala. No bloco de notas,
  título, padding, raio e × também escalam, então o texto não migra para o
  centro no zoom out nem para o canto no zoom in. A regressão
  `note_content_stays_proportional_when_zooming` mede o inset do conteúdo em
  duas escalas.
- O bloco de código do chat mantém a proporção no zoom. A causa era o
  `TextViewStyle` do vendor ignorar `code_block` no `PartialEq`: o bloco nunca
  era reparseado e ficava com o estilo da primeira leitura. A comparação agora
  inclui `code_block`/`is_dark`, e a view recria o estado do markdown por faixa
  de zoom para atualizar também o botão Copiar. Padding e raio escalam junto com
  a fonte.
- O scroll do chat preserva a posição ao mudar o zoom: metade do scroll
  continua na mesma fração; quem estava no fim continua no fim, sem pular para
  cima/baixo.
- `auri note write` e `auri note edit` voltam a atualizar a nota visível. O
  efeito do host usava um `update_window` aninhado na mesma janela e retornava
  `window_unavailable`; agora usa a janela viva que já atende o request. Vale
  para o pedido vindo do chat ou do terminal do agente.
- ↑/↓ no composer percorrem os prompts enviados; o slot depois do último é
  sempre o input vazio. Clicar no cartão foca o input do chat.
- Esc no chat interrompe o turno ativo e limpa a fila, exatamente como o botão
  de pausa.
- Enter no seletor de barra preenche e envia a linha destacada; com `/` vazio,
  manda o `/new` padrão em vez de enviar a barra sozinha. Sem nenhuma linha na
  lista, o Enter continua enviando o texto digitado (ex.: `/clear`).
- O chat abre com o modelo que o OMP já tem carregado: quando a sessão ainda não
  informou um `model_change`, o chip usa `modelRoles.default` do
  `~/.omp/agent/config.yml`. O valor é relido a cada abertura do chat, então
  trocar o modelo no OMP com o app aberto não deixa mais o chip no padrão
  antigo. O modelo da sessão continua tendo prioridade.
- O medidor de contexto conta os tokens de prompt que o OMP usa
  (`usage.contextTokens` ou `input + cacheRead + cacheWrite`) de todas as
  respostas do assistente, inclusive turnos de ferramenta, e ignora usage zero
  de turnos abortados. Antes, ele usava `totalTokens`, que somava a resposta e
  inflava a porcentagem; e uma sessão sem usage deixava o componente oculto em
  vez de mostrar o anel vazio em 0%.
- O chat do OMP volta a ler o JSONL da sessão e mostra só a resposta final do
  modelo. Mensagens intermediárias com `toolCall`, pensamentos, resultados de
  ferramenta e barra de status não viram mais bolha. O filtro por texto visível
  do terminal não distingue a narração (“vou ler o arquivo”) da resposta e
  descartava a resposta real depois da última caixa de ferramenta.
- `/clear` e uma sessão nova não deixam mais o chat preso ao histórico antigo:
  o cartão segue o breadcrumb do TTY, troca o arquivo lido e persiste o novo
  caminho para o próximo reload.
- O clique no campo de mensagem do chat passa a focar o input. O overlay do
  chat não repassa mais o mouse down para o cartão, que selecionava o cartão
  e impedia a digitação.
- Instalador não depende mais da API não autenticada do GitHub, que limita a
  60 requisições por hora por IP. A resolução da última release usa agora os
  URLs `releases/latest/download`, e `--no-desktop` não deixa o script terminar
  com código de erro após instalar.
- A roda do mouse sobre um cartão não move mais o canvas: lista de modelos,
  histórico do chat e terminal rolam até o fim com o cartão fixo. Antes, cada
  evento também aplicava pan no viewport, e o cartão saía de baixo do cursor
  antes de a lista chegar ao fim.
- O seletor de skills ganhou teclado: Tab insere a skill destacada (a primeira,
  sem navegação), ↑/↓ movem o destaque e o painel acompanha a seleção.
- O formulário do `ask` ganhou a linha **Outra resposta** em cada pergunta de
  escolha, além de aceitar `<input type=text>` e `<textarea>` no próprio chat.
  A resposta customizada é digitada no editor Other do TUI sem o `Space` que
  selecionava a opção. Os botões Cancelar e Enviar viraram pills arredondadas.
- O chip de modelo abre com o modelo selecionado no CLI mesmo quando a janela
  de eventos do histórico não inclui o começo da sessão. O catálogo de modelos
  é cacheado em `$XDG_CACHE_HOME/auro-canvas/models.json`: o seletor desenha o
  cache na hora e só o substitui quando o `omp models` trouxe um modelo novo,
  eliminando a tela de carregamento a partir da segunda execução.
- Anexos agora têm preview: imagens aparecem como miniaturas e documentos como
  um quadrado com logo e extensão acima do input. Ctrl+V com uma imagem grava
  os bytes em `$XDG_CACHE_HOME/auro-canvas/paste/` e anexa o caminho; o × de
  cada quadrado remove o caminho do input.
- No seletor de modelos, o modelo atual é o primeiro da lista do provedor e o
  provedor que o hospeda ganha um ponto accent de “em uso”, mesmo quando outro
  provedor está selecionado no drawer.
- A ilha direita e as ações **Conectar** trocaram o raio por `connect.svg`:
  dois cartões de agente ligados por uma corda com pulso. O raio permanece
  apenas como ícone de workspace.
- O medidor de contexto virou um anel de 40 px com 28 barras finas
  verde→dourado→vermelho à direita do input; sem a barra inferior e sem o
  ponto central. O chat inteiro escala com o zoom do canvas, mantendo as
  proporções em zoom out/in. O dado vem dos tokens de prompt
  (`usage.contextTokens` ou `input + cacheRead + cacheWrite`) da última
  resposta e do `contextWindow` do modelo.

### Adicionado

- Logos oficiais dos agentes no cabeçalho do cartão: OMP, Claude, Codex, Pi e
  OpenCode, monocromáticos em `currentColor`. Um badge ao lado do nome mostra
  `admin` (escudo) ou `member` (pessoa) e acompanha mudanças de papel feitas no
  formulário ou por `auri agent role`.
- Menções `@` no composer do chat: lista somente terminais, blocos de notas e
  agentes conectados ao cartão atual, inclusive conexões de outro workspace.
  Terminais inserem `@Nome`; notas inserem `@note #<id>`; agentes inserem
  `@Nome (#<id>)`. Toda menção prefixa `/skill:auri`, a skill compartilhada em
  `~/.agents`/`~/.claude`, para o agente ler o protocolo de comunicação. Itens
  desconectados ficam fora da lista até serem conectados.
- Comando `/new` no seletor de barra do chat: aparece antes das skills e
  começa uma nova sessão no cartão quando enviado. O comando vai verbatim ao
  OMP e não entra na fila de mensagens do transcript.
- Seletor de modelos no chat do OMP. A lista vem de `omp models --json` a cada
  abertura (nada hardcoded), com um drawer de provedores à esquerda, modelos
  à direita, scroll nas duas colunas e busca pill. A troca abre o model picker
  nativo do OMP (`Alt+P`) e envia o selector; escrever `/model` no PTY vira
  prompt, então não é usado. O modelo atual vem do evento `model_change`; o
  chip mostra `trocando…` até a confirmação.
- Formulário do tool `ask` dentro do chat: o HTML da pergunta é convertido em
  opções e o chat envia `↑/↓`, `Space`, `Tab` e `Enter` para o TUI. A resposta
  escolhida vira bolha do usuário e o formulário fecha com o `toolResult`.
  Perguntas com campo livre continuam pedindo o terminal.
- Indicador de atividade em um balão à esquerda do chat, com três pontos
  animados em onda e o status ao lado (`pensando…`, `usando <ferramenta>…`,
  `escrevendo…`). A linha de status acima do input foi removida.
- Botão de pausa à direita do composer, 40×40, com fundo vermelho e ícone
  branco de 20px. Ele envia Escape ao OMP (`app.interrupt`) ou o interrupt de
  terminal aos outros CLIs e limpa a fila de mensagens pendentes.
- Botão de anexar à esquerda do composer, 40×40 com ícone de 19px. O seletor
  nativo aceita vários arquivos de qualquer tipo (imagens, textos, documentos);
  os caminhos são inseridos no input, com aspas quando há espaços, para a CLI
  ler o que o terminal aceitar.

### Alterado

- As bolhas do chat seguem o estilo do WhatsApp: usuário em verde, agente em
  azul, canto com rabicho no topo e sem borda; anexo à esquerda, input no
  centro e pausa à direita. Os goldens visuais foram regravados.
- O input de busca de modelo usa o mesmo raio pill do input de texto do chat
  (`RADIUS_INPUT`), que passa a ser o padrão dos campos de texto da interface.
- O status do OMP voltou a nomear a ferramenta em execução e vive dentro do
  chat, sem ocupar a linha do composer.
- O chat do OMP reconstrói o histórico a partir do arquivo de sessão. A saída
  do terminal continua sendo a fonte apenas para agentes sem sessão
  estruturada, com extração do último bloco de resposta.
- Criar um novo cartão OMP inicia uma sessão nova com `omp`. A sessão só é
  retomada ao reabrir um cartão que já tem o caminho salvo (`omp --session`).
  Sem caminho salvo, o reload também começa do zero em vez de usar
  `omp --continue`.

## [v0.1.4] — 2026-11-26

### Adicionado

- Versão do programa no canto inferior esquerdo do drawer, com destaque em
  âmbar quando existe uma release verificada mais recente.
- Card de atualização no canvas mostra a versão disponível, a versão atual e
  a instrução de reiniciar o Auri para aplicar.

### Alterado

- OMP é o primeiro agente da lista e o ícone padrão de "Adicionar agente";
  Open Code vem em segundo. Os demais mantêm a ordem anterior.
- Cartões OMP retomam a sessão deixada no workspace em vez de abrir uma nova.
  O comando usa `omp --session <arquivo>` quando a sessão já foi identificada
  pelo breadcrumb do TTY; na primeira vez, `omp --continue` retoma a sessão
  mais recente do diretório. O caminho fica persistido no cartão.

## [v0.1.3] — 2026-11-26

### Corrigido

- O updater agora verifica o manifesto assinado a cada boot e uma vez por hora,
  mostrando um aviso no canvas quando existe versão mais recente. A aplicação
  continua acontecendo no boot, sem interromper a sessão atual.

## [v0.1.2] — 2026-11-26

### Corrigido

- Com nenhum espaço de trabalho carregado, o aviso no centro do canvas não deixa
  mais cartões, proxies e linhas de conexão salvos aparecerem atrás dele.
- Rolar para baixo (roda ou gesto) agora move o conteúdo dos agentes e do
  terminal para baixo; rolar para cima move para cima. A conversão do delta foi
  invertida tanto no canvas quanto no scrollback do terminal.

## [v0.1.1] — 2026-11-25

### Adicionado

- Logo principal do Auri (caranguejo com terminal), com fonte SVG e PNG em
  `canvas/icons/`, usado no bundle, na página do projeto e no instalador.

- Clique na linha tracejada abre um popover ancorado no ponto clicado com as
  duas pontas da ligação, a indicação bidirecional e o botão de remover.
- Regressões de UI com dispatch real do GPUI e estado visual `connection-popover`.
- Roda do mouse sobre terminais e agentes percorre o scrollback; em aplicativos
  que capturam a roda, o evento é repassado ao PTY.
  Uma barra lateral com thumb arrastável aparece quando há histórico.
- Menu de botão direito em terminal, agente e bloco de nota com atalho
  `Conectar`, equivalente ao raio da ilha.
- Atualização automática via GitHub Releases com o canal `GithubChannel` do
  Auro 0.1.0-alpha.15: manifest assinado verificado antes do download, staging,
  apply de AppImage e restart helper. O canal segue apenas releases estáveis
  (`include-prereleases = false`); a chave privada de release fica fora do
  repositório e a pública vai em `Auro.toml`. O estado fica em
  `$XDG_STATE_HOME/auri` (0700).
- Script `canvas/scripts/install-appimage.sh`: instala a release estável mais
  recente, verificando a assinatura Ed25519 do manifesto Auro, o SHA-256 e o
  tamanho de cada artefato antes de copiar o AppImage e o CLI para
  `~/.local/bin`, com entrada `.desktop` e ícone. Também aceita um AppImage
  local ou URL com `--manifest`/`--sha256`.
- A skill `auri` agora é instalada automaticamente em `~/.agents/skills/auri/`
  (e em `~/.claude/skills/` quando essa pasta já existe), além da cópia em
  cache usada via `AURI_SKILL`. Cópias gerenciadas são atualizadas no startup;
  uma skill escrita pelo usuário nunca é sobrescrita.
- Conexões entre workspaces: selecione a origem, troque de workspace pela barra
  lateral e clique no cartão ou nota de destino; a aresta fica qualificada
  pelos ids dos dois workspaces e aparece como proxy na ponta remota. O proxy
  pode ser arrastado como uma janela e sua posição fica salva no workspace que
  o exibe. A rope mostra os dois lados e o popover remove a aresta de ambos.
- `auri list`, `auri ask` e `auri check` aceitam endereços `id@workspace` ou
  `id@id-do-workspace`. O acesso exige a aresta desenhada, e o ask-back usa o
  remetente qualificado do envelope. Renomear um workspace preserva as arestas
  (que são ligadas por id); excluí-lo remove as arestas cruzadas dos demais.
- Atender um ask ou check em workspace ainda não carregado reconstrói os
  cartões dele em segundo plano sem mudar o workspace ativo.
- Arestas antigas com `from` inteiro continuam sendo lidas e são regravadas no
  formato objeto (`from: {"Card": id}`) quando o canvas é salvo novamente.

### Corrigido

- AppImage abre fora do diretório do projeto: as configurações de release passam
  a ser embutidas no binário e gravadas no estado privado do updater, sem
  depender de um `Auro.toml` ao lado do executável.
- O instalador registra o app com `Exec` absoluto, `TryExec` e ícone no tema
  `hicolor`, para que Auri apareça na busca do GNOME e do KDE Plasma sem
  depender de `~/.local/bin` estar no PATH.
- Trocar de workspace pela barra lateral durante o gesto de conexão não
  cancela mais a ligação, e clicar no cartão ou nota daquele workspace conclui
  a aresta cruzada em vez de criar uma aresta local inválida.
- Qualquer cartão selecionado — agente normal, administrador, terminal ou nota —
  pode iniciar uma conexão entre workspaces; o papel de admin continua valendo
  apenas para comandos CLI.
- O proxy da ponta remota fica logo acima do cartão remoto em vez de ser pintado
  atrás dele, e aparece visível mesmo quando os cartões das duas workspaces
  ocupam a mesma posição.
- O card proxy agora tem tamanho lógico fixo como os demais cartões e escala
  junto com o zoom, em vez de manter 240px na tela e parecer maior ao diminuir
  o zoom.
- Cancelar o gesto com um clique no canvas vazio limpa o destaque do cartão de
  origem mesmo quando ele ficou em outro workspace, devolvendo seleção e
  movimento normais.
- Cabeçalho dos cartões e blocos de nota reduzido para 32px, com alvo do ×
  de 24px e o mesmo tamanho de glifo de antes.
- Clique no raio da ilha não limpa mais a seleção antes de iniciar a conexão.
- A roda sobre o corpo do terminal não move mais o canvas; o movimento do
  canvas ficou só no cabeçalho/caminho do cartão.
- O renderer passou a respeitar `display_offset`, então o histórico realmente
  aparece na tela; tela alternativa sem mouse reporting não congela a roda.
  Um notch de roda vale três linhas em qualquer zoom, e um evento que não
  moveu o histórico segue para o scroll area em vez de ser engolido.
- A barra de scroll cobre toda a borda direita (incluindo a linha de caminho)
  e tem um token de drag próprio: arrastar o thumb não inicia redimensionamento
  nem deixa um drag de borda começado antes continuar ativo.
- Posicionar agente, terminal ou bloco de nota com o canvas deslocado usa a
  coordenada do clique: a origem pode ser negativa, então o cartão aparece onde
  foi clicado em vez de grudar na metade inferior da tela.

## [v0.1.0] — 2026-11-23

Primeira versão pública.

### Adicionado

- Canvas de agentes, terminais e notas com persistência em JSON.
- Conexões desenhadas pelo usuário, exibidas como linhas tracejadas.
- Papéis de agente normal e administrador, alternáveis no diálogo de edição.
- CLI local `auri` com `list`, `ask`, `check`, comandos de nota e comandos de
  criação/remoção para administradores.
- Skill `auri` distribuído no XDG cache com o padrão de ask-back.
- Cache por linha e damage tracking no renderer do terminal.
- Perfil `dev` com dependências otimizadas para manter fluidez em debug.
- Testes de regressão de código e suíte visual com goldens.
- Licença MIT e aviso de escopo/suporte em `NOTICE.md`.

### Corrigido

- Diálogo de edição do agente não abria após clicar em `Editar`.
- Aviso de workspace não carregado no canvas vazio.
- Badge de FPS removido da interface.
- Crash do `resize_axis` com limites invertidos ao redimensionar card pela
  borda esquerda/superior.
