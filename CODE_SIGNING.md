# Politique de signature

Les releases de Clipper seront signées gratuitement par [SignPath.io](https://signpath.io), avec un certificat de [SignPath Foundation](https://signpath.org) (candidature en cours).

## Ce qui est signé

Seuls `clipper.exe` et l'installeur `ClipperSetup-<version>.exe`, construits par la CI GitHub de ce dépôt à partir d'un commit de `main`. Rien n'est construit ni signé à la main.

## Rôles

| Rôle | Qui |
|---|---|
| Auteur et relecteur (*committer, reviewer*) | [HugoBernier](https://github.com/HugoBernier) |
| Validation des releases (*approver*) | [HugoBernier](https://github.com/HugoBernier) |

Le code est écrit avec l'aide de Claude Code (IA). Chaque changement passe par une pull request relue, testée et fusionnée par le mainteneur ; une release n'est signée qu'après sa validation.

## Vie privée

Ce programme ne transfère aucune information vers d'autres systèmes en réseau, sauf demande explicite de l'utilisateur ou de la personne qui l'installe ou l'utilise.

*This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it.*
