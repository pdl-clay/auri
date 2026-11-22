# Tokens visuais

- Todos os tokens visuais (cores, raios, espaçamento, tamanhos de fonte e dimensões de componentes) existem **apenas** em `canvas/ui/src/theme.rs`, o CSS global do projeto.
- Nenhum componente deve conter cores hexadecimais ou números mágicos de estilo; importe os tokens de `crate::theme`.
- Ao adicionar ou alterar um estilo, edite `theme.rs` e atualize os usos — nunca crie constantes locais.
