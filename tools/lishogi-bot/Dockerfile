# syntax=docker/dockerfile:1
# minase を lishogi の Bot アカウントとして動かすイメージ。
# ビルド文脈は minase リポジトリの `git archive <コミット>` であり、
# tools/lishogi-bot/build-image.sh がコミットを固定して本ファイルを実行する
# (docs/plans/lishogi-bot.md「接続の構成」)。

FROM rust:1.88-bookworm AS engine
WORKDIR /src
COPY . .
RUN cargo build --release --locked --bin minase

FROM python:3.11-slim-bookworm
# TheYoBots/Lishogi-Bot の固定コミット(2024年10月26日)。
ARG LISHOGI_BOT_COMMIT=17c16bc73b22fa6d56e0a412174c7c44993e619d
# 公開運用に使う minase のコミット。build-image.sh が渡す。
ARG MINASE_COMMIT
RUN apt-get update \
    && apt-get install -y --no-install-recommends git \
    && rm -rf /var/lib/apt/lists/* \
    && git clone https://github.com/TheYoBots/Lishogi-Bot /opt/lishogi-bot \
    && git -C /opt/lishogi-bot checkout --detach "$LISHOGI_BOT_COMMIT" \
    && pip install --no-cache-dir -r /opt/lishogi-bot/requirements.txt \
    && apt-get purge -y git && apt-get autoremove -y
RUN mkdir -p /var/log/lishogi-bot
COPY --from=engine /src/target/release/minase /opt/minase/minase
COPY tools/lishogi-bot/minase-lishogi /opt/minase/minase-lishogi
# 設定ファイルはイメージへ入れず、compose.yml が tools/lishogi-bot/config.yml を
# /opt/lishogi-bot/config.yml へ読み取り専用でマウントする。
WORKDIR /opt/lishogi-bot
ENV MINASE_COMMIT=${MINASE_COMMIT}
LABEL org.opencontainers.image.revision=${MINASE_COMMIT}
# 認証トークンは実行時に環境変数 LISHOGI_BOT_TOKEN で渡す。
ENTRYPOINT ["python3", "lishogi-bot.py"]
CMD ["-v", "--logfile", "/var/log/lishogi-bot/lishogi-bot.log"]
