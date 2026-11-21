# Changes

## 0.2.3

- Raccourcir les noms des tests sans modifier les assertions
- Corriger les données de test de l’historique du SDK de production et vérifier les limites configurées

## 0.2.2

- Le badge d'état affiche, après son icône, l'état et la durée, ou la raison une fois en pause ; il ne répète plus le mot Objectif
- Le panneau propose Suspendre ou Reprendre, et Terminer. Modifier est supprimé : `/goal <nouveau texte>` remplace l'objectif. Effacer devient Terminer, `/goal end`, et un objectif terminé apparaît comme Terminé dans l'historique
- Un tour d'objectif qui échoue est signalé une seule fois, par la notification de l'application ; l'objectif affiche toujours En pause · Erreur

## 0.2.1

- Un objectif que plus rien ne peut relancer ne reste plus en attente ou en cours : il s'affiche en pause lorsque le travail en arrière-plan attendu a été arrêté, que son résultat a été retenu à la limite ou que son dernier tour s'est terminé avant un redémarrage

## 0.2.0

- `/goal <texte>` démarre aussitôt et continue par des tours automatiques jusqu’à l’achèvement ; un message ou une pause restent dans l’objectif
- L’achèvement est une déclaration avec preuves qu’un tour distinct « Vérifier l’objectif » doit confirmer ; Rouvrir l’annule
- Arrêter met en pause et une réponse reprend ; Suspendre tient jusqu’à Reprendre ; un redémarrage, une bifurcation, un retour arrière ou le mode Plan mettent en pause avec leur motif
- Limites : 25 tours automatiques par défaut, budgets facultatifs de jetons et de temps, règle d’absence de progrès ; une limite atteinte conclut puis met en pause
- Trois outils en remplacent six ; vous seul modifiez, remplacez ou effacez l’objectif

## 0.1.10

- Icônes au trait rétablies dans la navigation et les outils

## 0.1.9

- Nouvelle icône du plugin

## 0.1.6

- Suspendre les abonnements des objectifs inactifs et suivre les changements d’état

## 0.1.4

- Ajout de la catégorie et de la couleur du catalogue
- Ajout des noms d’outils traduits
