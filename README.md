# KT3 Player for 3DS

[English](#english) | [日本語](#日本語)

## English

Player for `.kt3` files on the Nintendo 3DS (homebrew). The audio is decoded on the 3DS
itself with the reference implementation of [ktc](https://github.com/KEITO00/ktc).

### Features

- Plays loops at the loop points stored in the file, without a gap
- Shows the thumbnail, title and artist
  - Japanese text is shown with the system font.
- Plays files with separate vocal and instrumental tracks
  - Each track is decoded on its own CPU core.
- Keeps playing with the lid closed while headphones are connected

### Speed on Old 3DS

The values were measured with the speed test (`Y`). 1.00x means that decoding is exactly as
fast as playback.

| Audio | Speed |
|---|---|
| 29 kHz, stereo | 2.48x |
| 48 kHz, stereo | 1.70x |
| Vocal and instrumental, 44.1 kHz, stereo (two CPU cores) | 1.33x |

### Requirements

- A Nintendo 3DS with the Homebrew Launcher
- The DSP firmware at `sdmc:/3ds/dspfirm.cdc` (made with [DSP1](https://github.com/zoogie/DSP1))
- `.kt3` files made with KT3 Creator 1.3.1 or later, or `.ktc` files

### Use

1. Download `kt3-player-3ds.3dsx` from [Releases](https://github.com/KEITO00/kt3-player-3ds/releases) and copy it to `sdmc:/3ds/`.
2. Copy `.kt3` and `.ktc` files to `sdmc:/kt3/`.
3. Start KT3 Player for 3DS from the Homebrew Launcher.

| Button | Action |
|---|---|
| Up / Down | Select a song |
| Left / Right | Move by 6 songs |
| `A` | Play |
| `B` | Stop |
| `X` | Turn the loop on or off |
| `Y` | Speed test |
| `START` | Exit |

### Build

The following are required.

- devkitPro with devkitARM, libctru and 3dstools
- Rust nightly with `rust-src` (selected by `rust-toolchain.toml`)
- [cargo-3ds](https://github.com/rust3ds/cargo-3ds)
- libclang

Run the following command. The app is written to
`target/armv6k-nintendo-3ds/release/kt3-player-3ds.3dsx`.

```
sh build.sh
```

- On Linux and macOS, set the environment variable `DEVKITPRO` to the devkitPro folder.
- On Windows, run the command in Git Bash.
  - devkitPro must be installed in `C:\devkitPro`.
  - libclang is taken from the Python package `libclang` (`python -m pip install libclang`).
- The build has been tested only on Windows.

`vendor/ctru-rs` is a copy of [ctru-rs](https://github.com/rust3ds/ctru-rs) at commit
`9576b69`. Two lines of `ctru-sys/build.rs` are changed.

- The change lets the build find the libctru headers when the folder names in the path are
  separated by `\` (Windows).

### License

The code is licensed under the MIT License. `icon.png` and `vendor/ctru-rs` are not covered.
See [LICENSE](LICENSE).

## 日本語

`.kt3` を再生する、Nintendo 3DS 用のプレイヤー（homebrew）です。音声は、
[ktc](https://github.com/KEITO00/ktc) の参照実装を使用して、3DS 本体で復元します。

### 機能

- ファイルに保存されたループ位置での、途切れのないループ再生
- サムネイル、曲名、アーティスト名の表示
  - 日本語は、本体のフォントで表示します。
- ボーカルとインストが分かれたファイルの再生
  - それぞれを別の CPU コアで復元します。
- イヤホンの接続中の、ふたを閉じた状態での再生

### Old 3DS での速度

速度テスト（`Y`）で測定した値です。1.00x は、復元の速度が再生の速度と同じであることを表します。

| 音声 | 速度 |
|---|---|
| 29 kHz、ステレオ | 2.48x |
| 48 kHz、ステレオ | 1.70x |
| ボーカルとインスト、44.1 kHz、ステレオ（2 つの CPU コア） | 1.33x |

### 必要なもの

- Homebrew Launcher を導入した Nintendo 3DS
- `sdmc:/3ds/dspfirm.cdc` の DSP ファームウェア（[DSP1](https://github.com/zoogie/DSP1) で作成したもの）
- KT3 Creator 1.3.1 以降で作成した `.kt3`、または `.ktc`

### 使い方

1. [Releases](https://github.com/KEITO00/kt3-player-3ds/releases) から `kt3-player-3ds.3dsx` をダウンロードし、`sdmc:/3ds/` にコピーしてください。
2. `.kt3` と `.ktc` を `sdmc:/kt3/` にコピーしてください。
3. Homebrew Launcher から KT3 Player for 3DS を起動してください。

| ボタン | 操作 |
|---|---|
| 上 / 下 | 曲の選択 |
| 左 / 右 | 6 曲ずつの移動 |
| `A` | 再生 |
| `B` | 停止 |
| `X` | ループのオン / オフ |
| `Y` | 速度テスト |
| `START` | 終了 |

### ビルド

次のものが必要です。

- devkitARM、libctru、3dstools を含む devkitPro
- `rust-src` を含む Rust nightly（`rust-toolchain.toml` で選択されます）
- [cargo-3ds](https://github.com/rust3ds/cargo-3ds)
- libclang

次のコマンドを実行してください。アプリは
`target/armv6k-nintendo-3ds/release/kt3-player-3ds.3dsx` に作成されます。

```
sh build.sh
```

- Linux と macOS では、環境変数 `DEVKITPRO` に devkitPro のフォルダを設定してください。
- Windows では、Git Bash で実行してください。
  - devkitPro は、`C:\devkitPro` に導入されている必要があります。
  - libclang は、Python のパッケージ `libclang`（`python -m pip install libclang`）のものを使用します。
- ビルドを確認しているのは、Windows のみです。

`vendor/ctru-rs` は、[ctru-rs](https://github.com/rust3ds/ctru-rs) のコミット `9576b69` のコピーです。
`ctru-sys/build.rs` の 2 行を変更しています。

- パスのフォルダ名が `\` で区切られている場合（Windows）にも、libctru のヘッダーを見つけられるようにするためです。

### ライセンス

コードは、MIT ライセンスで公開しています。`icon.png` と `vendor/ctru-rs` は対象外です。
詳しくは [LICENSE](LICENSE) を参照してください。
