# Contribuindo

Auri é mantido como ferramenta pessoal. Issues e PRs são bem-vindos, mas não
há SLA nem promessa de revisão rápida — veja [NOTICE.md](NOTICE.md).

## Antes de começar

- Abra uma issue para bugs ou ideias; PRs grandes sem discussão prévia têm
  menos chance de serem aceitos.
- Use `rust-version` do manifesto como referência (1.90 no momento).
- O projeto roda localmente; não há backend, conta ou segredo necessário.

## Setup

```bash
cd canvas
cargo test --workspace
cargo run --release
```

Para desenvolvimento com hot reload, use `auro dev --dir canvas` se o Auro CLI
estiver instalado.

## Convenções

- Tokens visuais (cores, raios, espaçamento, dimensões) vivem apenas em
  `canvas/ui/src/theme.rs`. Nada de hex ou números mágicos de estilo nos
  componentes.
- O modelo persistido fica em `canvas/types` e precisa permanecer compatível
  com configs antigas: use `#[serde(default)]` em campos novos.
- O CLI é um cliente fino. Toda autorização e mutação acontece no host.
- Novos comportamentos visíveis devem vir com teste de código; mudanças na
  interface devem vir com golden atualizado em `canvas/tests/visual/`.

## Pull requests

- Um propósito por PR.
- Descreva o que mudou, como testou e quais são as limitações conhecidas.
- Rode `cargo test --workspace` antes de abrir. A suíte visual exige
  Wayland/niri, `grim` e `magick`; se não puder rodá-la, diga isso.
- Ao contribuir, você certifica o
  [Developer Certificate of Origin](https://developercertificate.org/) e pode
  assinar commits com `git commit -s`.

Não há CLA nem cessão de copyright.
