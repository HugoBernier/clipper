# Sécurité

## Signaler une faille

Utilisez le signalement privé de GitHub (compte GitHub requis) : onglet **Security and quality** du dépôt (dans le menu **•••** si la fenêtre est étroite) → **Report a vulnerability**. N'ouvrez pas de ticket public : la faille serait visible avant d'être corrigée.

Décrivez ce que vous avez fait, ce qui s'est passé et la version de Clipper. Vous aurez une réponse sous quelques jours ; c'est un projet personnel, pas une équipe d'astreinte.

## Versions corrigées

Seule la dernière release reçoit des correctifs.

## Ce qui compte comme faille

Tout ce qui contredit les promesses de la section [Sécurité et vie privée](README.md#sécurité-et-vie-privée), par exemple :

- Clipper envoie des données hors du PC, ou écrit un fichier sans appui sur le raccourci ;
- un fichier ou une page externe arrive à piloter la fenêtre de Clipper ;
- Clipper lit, renomme ou supprime un fichier hors du dossier des clips ;
- l'installeur ou l'app donne plus de droits qu'il n'en faut.

Ne sont pas des failles : une détection antivirus à tort (l'exe n'est pas signé) ou un bug sans conséquence sur vos données. Pour ceux-là, ouvrez un ticket normal.
