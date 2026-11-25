# Changelog

## [Não publicado]

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
  apply de AppImage e restart helper. `include-prereleases = true` permite o
  canal de teste; a chave privada de release fica fora do repositório e a
  pública vai em `Auro.toml`. O estado fica em `$XDG_STATE_HOME/auri` (0700).
- A skill `auri` agora é instalada automaticamente em `~/.agents/skills/auri/`
  (e em `~/.claude/skills/` quando essa pasta já existe), além da cópia em
  cache usada via `AURI_SKILL`. Cópias gerenciadas são atualizadas no startup;
  uma skill escrita pelo usuário nunca é sobrescrita.

### Corrigido

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
