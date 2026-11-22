# Drawer de espaços de trabalho — desenho

Data: 2026-11-22

## Intenção

O app já tem um canvas de terminais de agentes, mas tudo vive num único espaço. O usuário quer separar os agentes por projeto: cada espaço de trabalho aponta para uma raiz, e os terminais criados nele abrem nessa pasta. Um drawer lateral estreito mostra os espaços, deixa filtrar, criar e apagar, e troca o canvas ao selecionar um.

A referência visual é a imagem enviada pelo usuário: coluna escura de ~200 px, campo "Filtrar" no topo, botão "+", rótulo "ESPAÇOS DE TRABALHO" e linhas com ícone, nome e um monitor com o número de terminais. A linha ativa aparece realçada.

## O que o usuário confirmou

- O número ao lado do monitor é a quantidade de terminais abertos no espaço.
- Clicar num espaço troca o canvas para os terminais dele.
- O "+" abre um formulário com nome, ícone e raiz.
- A linha realçada é o espaço ativo.
- A lista persiste entre execuções.
- Apagar um espaço encerra todos os terminais dele e remove a linha.
- A raiz deve ser escolhida com seletor nativo de pasta, não campo de texto livre.
- Paleta de ícones fixa aprovada: estrela, pasta, raio, código, globo.
- A descrição completa do drawer foi revisada e aprovada.

## Goals

1. Drawer lateral sempre visível com busca, botão de criar e lista de espaços.
2. Criar espaço com nome, ícone da paleta e raiz escolhida por seletor nativo.
3. Apagar espaço encerrando seus terminais e persistindo a remoção.
4. Trocar de espaço sem encerrar os terminais de nenhum espaço.
5. Terminais criados num espaço abrem com `cwd` igual à raiz dele.
6. Persistir a lista e o espaço ativo entre execuções.

## Non-goals (primeira camada)

- Renomear, trocar ícone ou raiz de um espaço depois de criado.
- Reordenar espaços, cores por espaço, temas por espaço.
- Agrupar espaços, importar/exportar configuração, sincronizar.
- Detecção automática de repositórios git como raiz.
- Conectores, post-it, zoom/pan — continuam em camada futura do canvas.
- Segunda janela nativa só para o drawer.

## Anti-goals

- Encerrar terminais de outros espaços ao trocar ou apagar um espaço.
- Perder a saída de um terminal ao trocar de espaço — os agentes continuam vivos.
- Fazer o usuário digitar caminho como única forma de escolher a raiz.
- Abrir um diálogo de confirmação para apagar (decisão já tomada no app base).
- Adicionar um crate de diálogo ao projeto — o framework já tem a API.

## Restrições e evidências

- O app roda sobre auro `0.1.0-alpha.12`, com `gpui = "=0.2.2"` fixado pelo framework. Manter o pin exato no manifesto.
- O seletor de pasta **já existe** no release atual: `prompt_for_paths` do GPUI 0.2.2, reexportado como `auro::gpui::PathPromptOptions` + `cx.prompt_for_paths(...)`. Confirmado pelo líder da framework no [issue pdl-clay/auro#107](https://github.com/pdl-clay/auro/issues/107) e verificado no código local (`gpui-0.2.2/src/app.rs:1116`, `platform.rs:1330`, com backends Linux/macOS/Windows).
- Uso: `PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(...) }`. Retorno: `oneshot::Receiver<Result<Option<Vec<PathBuf>>>>`; `None` é cancelamento, `Err` é falha ao abrir o diálogo.
- Limitação real da API: não aceita diretório inicial. Cancelamento não deve gerar espaço nem erro.
- Linux usa XDG Desktop Portal; se o portal não estiver disponível, `Err` recebe fallback de campo de texto na hora — sem crate novo, sem mudança de pin.
- Runtime sem webview, HTML, CSS ou JavaScript.
- Este desenho assume o canvas de terminais da camada anterior presente neste branch.

## Decisões tomadas

Marcadas com **[seu]** quando vieram do usuário e **[minha]** quando são escolhas minhas.

1. **[seu]** Drawer lateral, filtro, "+", lista com ícone/nome/contador, seleção realçada.
2. **[seu]** Criar/apagar espaços; escolher ícone da paleta, nome e raiz; seletor nativo; persistência.
3. **[seu]** Apagar encerra os terminais do espaço e remove a linha; trocar de espaço troca o canvas.
4. **[seu]** Paleta: estrela, pasta, raio, código, globo.
5. **[minha]** Drawer sempre visível; recolhimento fica para camada futura.
6. **[minha]** Filtro case-insensitive por substring do nome; não filtra por caminho.
7. **[minha]** Configuração em `XDG_CONFIG_HOME` (ou `~/.config`)/`auro-canvas/workspaces.json`, escrita a cada mutação. Serialização com `serde`/`serde_json`, adicionadas de propósito — o scaffold as removera por serem usadas só no demo.
8. **[minha]** Um espaço inicial na primeira execução: nome "AURI", ícone estrela, raiz igual ao diretório de trabalho atual.
9. **[minha]** Formulário de criação expande dentro do drawer, sem janela modal própria; o seletor de pasta é o único diálogo nativo.
10. **[minha]** Validação: nome não vazio e diretório existente; erros aparecem em texto no próprio formulário.
11. **[minha]** Depois de apagar o espaço ativo, ativa o primeiro restante — ou nenhum, se a lista esvaziar.
12. **[minha]** Identificadores de espaço e terminal são estáveis e serializados; ao carregar, terminais não são recriados automaticamente — apenas a lista de espaços e o ativo voltam.
13. **[minha]** Terminais de espaços inativos continuam rodando; o canvas mostra só os do espaço ativo.
14. **[minha]** Sem espaço ativo (lista vazia), não existe espaço implícito: o botão "Adicionar terminal" fica desabilitado e o canvas mostra uma dica para criar um espaço. Nenhum terminal é criado sem raiz definida.
15. **[minha]** Se a raiz de um espaço persistido deixar de existir ou ficar inacessível, o espaço permanece na lista com marcador de inválido; ao criar terminal nele, o erro aparece no texto de erro existente do canvas. Não descarto espaço automaticamente.
16. **[seu]** Se `workspaces.json` estiver ausente, ilegível ou malformado: preservar o arquivo e tentar recuperar. Nunca apagar ou sobrescrever silenciosamente.
17. **[minha]** Recuperação: ao falhar o parse, manter os bytes originais intactos e mostrar no drawer um estado de recuperação com o erro. Tentar um salvage tolerante das entradas completas; se conseguir, carregar o que foi salvo e exibir aviso "recuperado". Se não conseguir, seguir com o espaço inicial em memória e oferecer "Recomeçar", que copia o arquivo ruim para `workspaces.json.bad-<timestamp>` antes de escrever uma config nova.

## Forma da primeira camada

O drawer ocupa uma coluna fixa à esquerda do canvas. No topo, campo de filtro e botão "+". Abaixo, o rótulo da seção e a lista rolável. Cada linha mostra o ícone, o nome e um ícone de monitor seguido da contagem de terminais daquele espaço. Clique seleciona e troca o canvas.

Ao clicar "+", o drawer expande um formulário com nome, cinco ícones selecionáveis e um botão de escolher raiz; o botão chama `cx.prompt_for_paths` e exibe o caminho escolhido. Cancelar o seletor não cria nada. Confirmar com nome e raiz válidos cria o espaço, salva o JSON, seleciona o novo espaço e fecha o formulário.

Ao apagar, os terminais daquele espaço têm suas entidades dropadas — o `PtySession::Drop` existente encerra o grupo de processos —, a linha sai da lista e o JSON é reescrito. Espaços inativos guardam seus terminais em memória; trocar de espaço apenas esconde e mostra conjuntos.

## Camadas seguintes

- Editar nome, ícone e raiz de um espaço existente; reordenar.
- Recolher o drawer e atalhos de teclado.
- Restaurar terminais automaticamente ao reabrir um espaço.
- Conectores, post-it, zoom/pan.

## Riscos a verificar cedo

- `prompt_for_paths` depende do XDG Desktop Portal em Wayland; confirmar o comportamento real de `Err` e do fallback na sessão do usuário.
- API não aceita diretório inicial — a UX precisa deixar isso claro no formulário.
- Persistência com `serde_json` reintroduz dependências removidas na camada anterior; confirmar que o pin do auro e o build continuam limpos.
- Trocar de espaço com terminais vivos exige que o canvas mantenha entidades de todos os espaços; conferir que o drop acontece só no apagar.

## O que fica para o builder

Estrutura de tipos e entidades, API exata do formulário e do desenho do drawer em GPUI, tratamento dos retornos do `prompt_for_paths`, esquema do JSON, testes e ordem de implementação. Este documento fixa o **quê** e **por quê**, não a API exata de cada chamada.
