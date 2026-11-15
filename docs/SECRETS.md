# Настройка владельцем

В GitHub откройте Settings → Secrets and variables → Actions. Токены вводите только там, не в исходниках или Issues.

## Публичный VibeCast

Variable `VIBECAST_BOT_USERNAME` — имя бота без @. Для Free release персональный токен не нужен: workflow использует штатный GITHUB_TOKEN с Contents write.

Actions → Free Windows release → Run workflow → версия, например 0.13.3. Альтернатива — отправить тег v0.13.3. Версия обновления должна быть больше установленной.

## Закрытый VibeCast-Pro

Сервер оплаты, платные персонажи и полные инструкции находятся в закрытом репозитории VibeCast-Pro.

| Secret | Назначение |
|---|---|
| TELEGRAM_BOT_TOKEN | Отдельный бот VibeCast от BotFather, только сервер |
| LICENSE_SIGNING_KEY_HEX | Постоянный Ed25519 seed: 32 байта в hex |
| PUBLIC_RELEASE_TOKEN | Fine-grained token только для публичного VibeCast, Contents read/write; загрузка готового Pro |
| DEPLOY_HOST | IP или имя VPS |
| DEPLOY_USER | root для изолированного установщика сервиса |
| DEPLOY_SSH_KEY | Отдельный приватный SSH-ключ развёртывания |
| DEPLOY_HOST_KEY | Проверенная полная строка known_hosts для VPS |

Variables закрытого репозитория: VIBECAST_BOT_USERNAME, VIBECAST_API_URL (отдельный HTTPS-домен без пути), TERMS_URL (условия покупки), SUPPORT_URL (контакт поддержки по HTTPS), ADMIN_USER_IDS (Telegram ID через запятую).

Deploy использует environment production. Задавайте параметры на уровне репозитория либо environment; параметры сборки Pro должны быть доступны и release workflow. В Windows попадает только публичный ключ проверки лицензий.

## Порядок запуска

1. Создать бота, опубликовать условия покупки и контакт поддержки.
2. В закрытом проекте выполнить `python tools/generate_license_key.py`. Сохранить .secrets/license.env в резервную копию, значение ключа перенести в Secret. Не менять его при обновлениях.
3. Настроить SSH и параметры, запустить Deploy isolated VibeCast service.
4. Подключить отдельный HTTPS-хост API по примеру закрытого проекта. Конфигурацию и службы Nover не заменять.
5. Выпустить Free, затем Pro той же версии. Pro workflow загрузит только бинарники в публичный Release.
6. Проверить реальную оплату, выдачу кода, активацию одного ПК, запрет второго и обновление обеих редакций. До этого приём платежей не считается запущенным.

GitHub Secrets не являются хостингом: бот работает отдельным процессом на VPS с собственной базой. Без введённых параметров платёжный сервис не запускается.

Документация: [Secrets](https://docs.github.com/en/actions/security-for-github-actions/security-guides/using-secrets-in-github-actions), [Telegram Stars](https://core.telegram.org/bots/payments-stars), [Release assets и digest](https://docs.github.com/en/rest/releases/assets).
