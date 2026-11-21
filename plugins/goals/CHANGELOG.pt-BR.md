# Changes

## 0.2.3

- Encurtar os nomes dos testes sem alterar as asserções
- Corrigir os dados de teste do histórico do SDK de produção e verificar os limites configurados

## 0.2.2

- O indicador de status mostra, após o ícone, o estado e o tempo, ou o motivo quando pausada; ele não repete mais a palavra Meta
- O painel tem Pausar ou Retomar e Encerrar. Editar foi removido: `/goal <novo texto>` substitui a meta. Limpar agora é Encerrar, `/goal end`, e uma meta encerrada aparece como Encerrada no histórico
- Um turno da meta que falha é notificado uma única vez, pela notificação do próprio aplicativo; a meta continua mostrando Pausada · Erro

## 0.2.1

- Uma meta que nada pode retomar não fica mais em espera ou em execução: ela aparece pausada quando o trabalho em segundo plano esperado foi interrompido, quando o resultado ficou retido no limite ou quando o último turno terminou antes de uma reinicialização

## 0.2.0

- `/goal <texto>` começa na hora e continua em turnos automáticos até concluir o objetivo; uma mensagem ou uma pausa continuam dentro do objetivo
- Concluir é uma declaração com evidências que um turno separado “Verificar objetivo” precisa confirmar; Reabrir desfaz
- Parar pausa e uma resposta retoma; Pausar vale até Retomar; reinício, bifurcação, retrocesso ou modo Plano pausam com o motivo
- Limites: 25 turnos automáticos por padrão, orçamentos opcionais de tokens e tempo e uma regra de falta de progresso; ao atingir um limite, o trabalho é encerrado e pausado
- Três ferramentas substituem seis; só você edita, substitui ou limpa o objetivo

## 0.1.10

- Ícones de contorno restaurados na navegação e nas ferramentas

## 0.1.9

- Novo ícone do plugin

## 0.1.6

- Manter objetivos sem uso inativos e observar apenas mudanças de estado

## 0.1.4

- Adiciona categoria e cor do ícone ao catálogo
- Adiciona nomes de ferramentas traduzidos
