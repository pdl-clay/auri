# Tokens visuais

- Todos os tokens visuais (cores, raios, espaçamento, tamanhos de fonte e dimensões de componentes) existem **apenas** em `canvas/ui/src/theme.rs`, o CSS global do projeto.
- Nenhum componente deve conter cores hexadecimais ou números mágicos de estilo; importe os tokens de `crate::theme`.
- Ao adicionar ou alterar um estilo, edite `theme.rs` e atualize os usos — nunca crie constantes locais.

# Regressões: todo bug corrigido ganha um teste

- Nenhum bug é considerado corrigido sem um teste de regressão que **falha no código anterior à correção** e passa depois.
- Bug de estado, dados ou geometria pura: teste em `canvas/ui/src/regressions.rs`, exercitando o caminho real (não uma reimplementação da lógica).
- Bug de UI, evento de ponteiro, layout ou pixel: adicione um estado em `tests/visual/golden/` que reproduza o gesto exato do usuário, via `scripts/visual-regression.sh --record`. Teste unitário não substitui isso, porque o defeito está na fiação dos eventos.
- Bug que já voltou uma vez: primeiro estenda o teste existente com o caso que faltava, confirme que ele falha, e só então conserte o código.
- Nunca apague, relaxe ou re-grave um golden para "fazer passar" sem antes explicar a mudança visual pretendida.
