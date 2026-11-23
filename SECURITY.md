# Segurança

## Versões suportadas

Apenas o HEAD da branch principal recebe correções. Não há suporte a versões
antigas, backports ou divulgação coordenada longa.

## Reportando

- Para vulnerabilidades: use o canal privado de
  [Security Advisories](https://github.com/pdl-clay/auri/security/advisories/new)
  do repositório, se disponível.
- Para outros problemas: abra uma issue normal. Não cole tokens, caminhos
  privados, prompts ou logs com dados sensíveis.

## Modelo de confiança (resumo)

- O CLI fala com o host por um Unix socket local com permissão `0600`.
- Cada PTY de agente recebe um token aleatório de 256 bits no ambiente; o
  token é revogado ao remover o card, descarregar o workspace ou reiniciar.
- Toda ação passa por `permissions::authorize`, que relê o bit de
  administrador do estado persistido a cada requisição.
- Nomes ou títulos não concedem autoridade; apenas conexões desenhadas pelo
  usuário e o papel do agente.
- O host nunca interpola texto do request em um shell.

Auri não é um serviço de produção e não deve ser exposto a agentes ou usuários
não confiáveis.
