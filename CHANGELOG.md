# Changelog

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
