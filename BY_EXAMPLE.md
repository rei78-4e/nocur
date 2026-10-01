# nocur by example

このページでは `replace`、`delete`、`insert`、`trim`、`lines`、`filter`、`map` を順に試します。

## 起動してプレビューする

```sh
cargo run -- examples/practice.txt
```

入力欄に変換を書くと、上側に結果が表示されます。入力はまだ確定されていないので、変換を直してプレビューを確認できます。

## 文字を置き換える・消す

```text
replace(/foo/, "nocur")
```

`foo` に一致する部分をすべて `nocur` に置き換えます。練習テキストでは、TODO行とnote行の `foo` が置き換わります。

```text
delete(/TODO: /)
```

一致した文字を削除します。TODO行のラベルだけを消し、タスクの文は残します。

## 行の前に挿入する

```text
insert(1, "# Tasks\n")
```

数値は1始まりの行番号で、指定した行の前に挿入します。`gg` は先頭行、`G` は最終行の前です。`insert(G, "# Last task follows\n")` は最後のタスクの直前に行を追加します。文字列に改行を指定しない場合、改行は自動では付きません。

## 空白を整える

```text
trim()
```

Buffer全体の先頭と末尾の空白を取り除きます。各行の空白を整えるには `map` と組み合わせます。

```text
map(trim())
```

練習テキストの `item:` 行は、それぞれの行頭のインデントが取り除かれます。`map` は行末の改行を維持します。

## 行を選ぶ

```text
lines(1..3)
```

`lines` は1始まり、両端を含む範囲で行を取り出します。これは練習テキストの先頭3行を返します。

```text
filter(/^TODO:/)
```

`filter` は正規表現に一致する行だけを残します。上の例では `TODO:` で始まる行を残します。

## Pipelineを組み合わせる

操作は `|>` でつなぎます。左から順に評価されます。

```text
filter(/^TODO:/)
|> map(replace(/^TODO: /, "") |> trim())
```

`TODO:` 行を選び、各行のラベルを消して空白を整えます。結果は次の3行です。

```text
rename foo in the guide
review the error message
publish the guide
```

## 確定して保存する

プレビューを確認したら `Ctrl+X` でHistoryに追加します。`Ctrl+S` は変換を確定して保存まで行います。保存先は既定で `examples/practice.txt.edited` です。`F3` または `Ctrl+R` でHistory一覧を開き、過去の状態を選べます。

同じPipelineはTUIを起動せずに実行できます。

```sh
cargo run -- examples/practice.txt --eval 'filter(/^TODO:/) |> map(replace(/^TODO: /, "") |> trim())'
```
