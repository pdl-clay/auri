# Roadmap

Este é um plano de intenção, não uma promessa. Auri é uma ferramenta pessoal;
itens só entram quando eu preciso deles ou quando alguém contribui.

## v0.1.0 — entregue

- Canvas com cartões de agente, terminal e nota; drag, resize, zoom e
  persistência.
- Conexões desenhadas pelo usuário com linha tracejada, bidirecionais e
  persistidas por workspace.
- Papéis normal/administrador com toggle no diálogo de edição.
- CLI `auri` sobre socket Unix local com token por PTY: `list`, `ask`,
  `check`, notas, criação/remoção de cartões (admin) e `skill`.
- Skill distribuído no XDG cache com padrão de ask-back.
- Renderer de terminal com cache por linha e damage tracking.
- Suítes de regressão de código e visual (goldens).

## Em consideração (sem prazo)

- Nome persistido para notas e endereçamento mais estável no CLI.
- Polir papercuts do CLI: validar flags por verbo, `--` para argumentos que
  começam com `-`, timeout de leitura e erros mais específicos.
- Honrar ou rejeitar `--name` em `terminal new` / `note create`.
- Rollback de notas quando o efeito ao vivo falha.
- Fechar menus/dialogs ao entrar no modo conexão e tratar botão direito.
- Suportar nomes/caminhos com caracteres de controle em todos os pontos de
  entrada.
- Adaptador MCP opcional sobre o mesmo CLI (não é prioridade).
- Conexões entre workspaces e alcance transitório (não planejado para já).
- Suporte explícito a macOS/Windows (o transporte hoje é Unix socket).

## Fora de escopo por decisão

- Chat, histórico de mensagens ou balões no canvas.
- A2A/HTTP, service discovery, push notification.
- Auto-conexão entre agentes — só o usuário conecta.
- Ser um substituto ou produto oficial do Maestri.
