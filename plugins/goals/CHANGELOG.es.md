# Changes

## 0.2.3

- Acortar los nombres de las pruebas sin cambiar las aserciones
- Corregir los datos de prueba del historial del SDK de producción y verificar los límites configurados

## 0.2.2

- La insignia de estado muestra, tras su icono, el estado y el tiempo, o el motivo una vez en pausa; ya no repite la palabra Objetivo
- El panel tiene Pausar o Reanudar y Finalizar. Se elimina Editar: `/goal <texto nuevo>` reemplaza el objetivo. Borrar pasa a ser Finalizar, `/goal end`, y un objetivo finalizado aparece como Finalizado en el historial
- Un turno del objetivo que falla se notifica una sola vez, con la notificación de la aplicación; el objetivo sigue mostrando En pausa · Error

## 0.2.1

- Un objetivo que nada puede reactivar ya no se queda en espera o en ejecución: se muestra en pausa cuando el trabajo en segundo plano que esperaba se detuvo, su resultado quedó retenido en el límite o su último turno terminó antes de un reinicio

## 0.2.0

- `/goal <texto>` empieza de inmediato y continúa con turnos automáticos hasta completar el objetivo; un mensaje o una pausa siguen dentro del objetivo
- Completar es una declaración con pruebas que un turno aparte «Verificar objetivo» debe confirmar; Reabrir lo deshace
- Detener pausa y una respuesta reanuda; Pausar se mantiene hasta Reanudar; un reinicio, una bifurcación, un retroceso o el modo Plan pausan con su motivo
- Límites: 25 turnos automáticos por defecto, presupuestos opcionales de tokens y tiempo, y una regla de falta de progreso; al alcanzar un límite se cierra el trabajo y se pausa
- Tres herramientas sustituyen a seis; solo tú editas, reemplazas o borras el objetivo

## 0.1.10

- Iconos de contorno restaurados en la navegación y las herramientas

## 0.1.9

- Nuevo icono del plugin

## 0.1.6

- Mantener inactivos los objetivos sin uso y observar solo los cambios de estado

## 0.1.4

- Añade categoría y color de icono al catálogo
- Añade nombres de herramientas traducidos
