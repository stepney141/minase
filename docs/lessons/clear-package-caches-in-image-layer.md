# コンテナイメージはパッケージ管理のキャッシュを同じ層で消し、ビルド直後に大きさを測る

## 症状

AlphaZero型探索の学習と推論に使うコンテナイメージ（`tools/train/zero/Dockerfile`）を作業機で初めてビルドしたところ、展開後の大きさが46.8 GBになり、作業機のディスクの空きが127 GBから44 GBまで減った。
README.mdの見込みは約15 GBだった。

## 原因

`uv sync` が依存をキャッシュディレクトリ（`UV_CACHE_DIR`）へ展開してから仮想環境へ複製する（`UV_LINK_MODE=copy`）ため、仮想環境8.8 GBとほぼ同じ内容のキャッシュ12 GBがイメージの層に残った。
さらに、Dockerのビルドキャッシュにも同じ層の複製が残り、ビルドをやり直すたびに数十GBが積み上がった。
`docker buildx prune` のフィルタ（`id=`、`description~=`）はこの環境では一致せず、個別の層だけを消すことはできなかった。

## 以後の規則

パッケージ管理（uv、pip、apt、cargo）を使う `RUN` では、導入と同じ層の中でキャッシュを消す（`uv cache clean`、`rm -rf /var/lib/apt/lists/*` など）。キャッシュを消すとディレクトリ自体が消える場合は、後続の `chmod` などが失敗しないよう作り直す。
イメージを作る前に `docker system df` で空きと回収可能な領域を見て、ビルド直後に `docker image ls` で大きさを見込みと比べ、見込みの2倍を超えていたらレジストリへ送る前に原因を調べる。
ビルドキャッシュの回収は、個別の層ではなく `docker builder prune` で行い、他のイメージのキャッシュも消えるので利用者の判断で実行する。

## 出典

- [plans/alphazero.md](../plans/alphazero.md) の「実行環境はコンテナイメージにまとめる」
- `tools/train/zero/Dockerfile` と `tools/train/zero/README.md`
