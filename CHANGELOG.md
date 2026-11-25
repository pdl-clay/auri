# Changelog

## [v0.1.1] — 2026-11-25

### Adicionado

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
