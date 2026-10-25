# CEF UI protection + auto-download (patch notes)

Apa yang berubah: `cef/assets/*.html` (dulu disk polos, bisa dibuka/edit
siapa saja) sekarang dikemas jadi satu file terenkripsi (`ui.bin`) yang
**otomatis di-download dari server** saat player konek — mirip alur
download model 0.3DL yang kamu minta — lalu didekripsi hanya di memori oleh
plugin client. Tidak ada lagi file `.html` polos yang perlu dikirim ke
player.

## Status jujur

- Kode sudah lengkap di semua lapisan (protokol, client, server) dan sudah
  saya review manual baris-per-baris (tipe data, borrow checker, pola API
  crate yang dipakai).
- **Belum pernah benar-benar dicompile.** Dua alasan: (1) plugin client
  target-nya Windows + D3D9, butuh toolchain MSVC + CEF binary distribution
  ~280MB yang tidak ada di sandbox ini; (2) sandbox ini **tidak punya akses
  ke registry crates.io** sama sekali (DNS-nya diblok di level jaringan,
  beda dari npm/pip yang kebetulan jalan) — jadi bahkan `cargo check` untuk
  bagian server Linux pun tidak bisa saya jalankan di sini.
- Jadi: **build pertama lewat GitHub Actions kamu (`make-package`) adalah
  langkah WAJIB**, bukan opsional, sebelum kamu coba di server production.
  Kalau CI gagal compile, kirim log errornya ke saya — biasanya perbaikan
  cepat (typo tipe, field, dsb), bukan rombak ulang.

## Alur lengkap (sudah termasuk perbaikan race-condition)

1. `scripts/pack_ui_assets.py` sekali tiap UI berubah → hasil `ui.bin`
   (bundel terenkripsi) + `ui_key.hex` (kunci AES-256, **rahasia server**)
   + `ui.version`.
2. `ui.bin` + `ui_key.hex` ditaruh di server, sejajar plugin CEF (default
   `cef/ui.bin`, `cef/ui_key.hex`, bisa diubah lewat `server.cfg`:
   `cef_ui_bundle` / `cef_ui_key`).
3. Plugin server load bundel itu sekali saat start.
4. Player konek, handshake CEF sukses (`JOIN_RESPONSE`) → server kirim
   `AssetManifest` (versi, sha256, ukuran, **kunci AES**) lewat socket yang
   sudah TLS 1.3 (QUIC, lihat `network/src/client.rs`).
5. **Server MENAHAN dulu** sinyal "player siap CEF" ke gamemode (yang
   memicu `cef_create_browser` pertama kali) — ini perbaikan dari revisi
   sebelumnya, lihat bagian "Perbaikan race-condition" di bawah.
6. Client cek cache lokal per-server (`cef/ui-cache/<ip_port>/ui.bin`).
   Cocok → langsung pakai. Beda/tidak ada → `RequestAssetBundle`, server
   stream `AssetChunk` 32 KB per potongan, client verifikasi sha256, tulis
   ke disk (**tetap terenkripsi**), dekripsi di RAM.
7. Client kirim `AssetBundleReady` ke server → server BARU kirim sinyal
   "player siap CEF" ke gamemode → `cef_create_browser` mulai dipanggil →
   dijamin bundel sudah ada saat browser pertama minta file.
8. CEF browser minta file lewat `sampcef://assets/...` → handler scheme
   ambil byte dari bundel di memori (bukan `fs::read` disk lagi).

## Perbaikan race-condition (penting buat "jangan freeze/blank")

Desain awal saya kemarin punya celah: server bisa saja menyuruh gamemode
mulai `cef_create_browser` (misal buat HUD) **sebelum** client selesai
download bundel pertama kali — hasilnya HUD/menu pertama kali muncul blank
sampai ada trigger lain yang reload browser itu. Bukan crash, tapi bug
yang kelihatan kayak CEF rusak.

Saya tambal dengan handshake baru:
- Paket baru `AssetBundleReady` (client → server) — dikirim begitu bundel
  terpasang (baik dari cache maupun habis download).
- Server **menahan** `PlayerConnected` (pemicu gamemode mulai bikin
  browser) sampai ack ini diterima, KECUALI server memang tidak
  dikonfigurasi pakai bundel sama sekali (mode lama/dev, langsung jalan
  seperti biasa, 100% backward compatible).
- **Fail-open 60 detik**: kalau ack tidak pernah datang (koneksi sangat
  lambat, atau player masih pakai `.asi` versi lama yang belum paham
  paket ini), server tetap lanjutkan setelah 60 detik — supaya pemain
  tidak stuck permanen, cuma CEF-nya telat muncul/tidak muncul untuk sesi
  itu. Lihat `server-core/src/server.rs::check_bundle_timeouts`.

## File yang diubah/ditambah

**Protokol:** `proto/packets.proto` + `messages/src/proto/packets.rs` +
`messages/src/packets.rs` — 4 pesan baru: `AssetManifest`,
`RequestAssetBundle`, `AssetChunk`, `AssetBundleReady` (PacketId 17-20).
(`.proto` diupdate juga, tapi generated Rust-nya saya tulis tangan persis
mengikuti pola existing — tidak ada `protoc`/`pb-rs` di sandbox ini.)

**Client (`client/`):**
- `client/Cargo.toml` — tambah `aes-gcm`, `sha2`
- `client/src/browser/asset_bundle.rs` **(baru)** — parser container
  terenkripsi + dekripsi per-file di memori (AES-256-GCM)
- `client/src/browser.rs` — daftarkan module baru
- `client/src/browser/assets_scheme.rs` — baca dari bundel dulu, fallback
  ke disk polos kalau bundel belum ter-load (dev convenience)
- `client/src/network.rs` — handle `ASSET_MANIFEST`/`ASSET_CHUNK`, kirim
  `RequestAssetBundle` + `AssetBundleReady`
- `client/src/app.rs` — cek cache per-server, download, rakit chunk,
  verifikasi hash, install bundel, kirim ack; bundel di-clear saat
  disconnect; `cef/assets/` auto dibuat kalau belum ada (lihat bawah)
- `client/src/utils.rs` — `ui_bundle_path_for(addr)` (per-server, lihat
  perbaikan cache sebelumnya)

**Server plugin SA-MP klasik** (sesuai screenshot GitHub Actions
`davidanderson013/samp-cef`):
- `server-core/src/client.rs` — state `bundle_pending` + timestamp per
  client
- `server-core/src/server.rs` — simpan bundel di memori, kirim
  `AssetManifest`, tahan `PlayerConnected` sampai `AssetBundleReady` atau
  timeout 60 detik, layani `RequestAssetBundle` dengan stream `AssetChunk`
- `server-core/src/lib.rs` — `ServerCore::load_asset_bundle(...)` baca
  `ui.bin`+`ui_key.hex` sekali saat start
- `server/src/lib.rs` — panggil itu saat plugin start, path dari
  `server.cfg` (opsional, ada default)

**open.mp** (kalau kamu juga build target ini): fungsi bridge
`load_asset_bundle` sudah ada di sisi Rust
(`openmp-component/src/lib.rs`), **belum tersambung ke C++** — file
component C++-nya tidak ada di upload kamu, kirim kalau perlu.

**Build tool (baru):** `scripts/pack_ui_assets.py` — bikin
`ui.bin`/`ui_key.hex`/`ui.version` dari folder assets biasa. Butuh
`pip install cryptography`.

**Migrasi client (baru):** `client-migration/Iternal_CEF_Migrate.bat` +
`README.md` — jalan di PC player, lihat bagian "Migrasi" di bawah.

## Setelah di-build (urutan deploy)

1. `python3 scripts/pack_ui_assets.py path/ke/cef/assets path/output`
2. Copy `ui.bin` + `ui_key.hex` ke server, sejajar plugin CEF
3. Restart/reload plugin server
4. **Build client** lewat GitHub Actions kamu, download artifact `cef.zip`
5. Di 1 PC test: ikuti `client-migration/README.md` buat jalankan
   `Iternal_CEF_Migrate.bat` (lihat detail di bawah)
6. Baru lakukan seluruh checklist testing di bawah sebelum dibagikan ke
   semua player

## Migrasi dari sistem lama ke sistem baru

`client-migration/Iternal_CEF_Migrate.bat` otomatis: tutup proses
GTA/CEF, backup `cef/assets` lama ke `cef_assets_backup_old` (bukan
dihapus permanen, buat jaga-jaga rollback), bersihkan cache CEF lama
(`cache`, `GPUCache`, `user_data`), timpa semua file plugin dengan build
baru dari folder `cef_update`, lalu hapus folder itu. Detail lengkap +
cara siapkan folder `cef_update` ada di `client-migration/README.md`.

Catatan penting: saya tambahkan `create_dir_all(cef/assets)` otomatis di
startup client (`App::new()`), supaya walau foldernya dihapus/dibackup
total oleh script migrasi, plugin tetap jalan normal next launch — sistem
resolusi URL lama (bukan buatan saya) butuh folder itu *ada* di disk buat
validasi path, walau isinya sudah tidak dipakai lagi untuk serving konten.

## Checklist testing — fokus crash / freeze / spawn / callback

Lakukan ini urut, di PC test dulu, **sebelum** dibagi ke semua player.
Pakai beberapa kondisi jaringan kalau bisa (WiFi kampus/warnet yang lemot,
bukan cuma koneksi kantor yang kencang) karena race-condition biasanya
baru kelihatan di koneksi lambat.

**A. Sesi pertama / cold cache (paling kritis — ini jalur baru semua)**
1. Jalankan `Iternal_CEF_Migrate.bat`, pastikan `cef/ui-cache` kosong
   (belum pernah download)
2. Buka GTA SA, konek ke server
3. Amati: apakah GTA **freeze/hang** selama proses download berlangsung?
   (Harusnya tidak — tapi kalau bundel kamu makin besar ke puluhan MB, ada
   kemungkinan hitch singkat sekali saat file final ditulis+didekripsi,
   lihat "Catatan performa" di bawah)
4. Tunggu sampai spawn selesai — **apakah HUD muncul normal, bukan blank
   putih/hitam?** (ini yang tadinya rawan race-condition)
5. Cek `cef_client.log` (ada di folder GTA SA) — harus ada baris
   `"CEF UI asset bundle downloaded and installed"`
6. Buka semua menu/fitur CEF yang biasa dipakai: inventory, phone, faction
   panel, admin duty, character creator, dsb — satu-satu, pastikan tidak
   ada yang blank atau CEF-nya tidak muncul sama sekali
7. **Tidak boleh ada crash/CTD** (crash to desktop) di titik mana pun pada
   proses ini

**B. Sesi kedua / warm cache (jalur paling sering dipakai player)**
1. Disconnect, reconnect ke server yang sama (tanpa hapus cache)
2. Harus langsung pakai cache (`cef_client.log`: `"using cached copy"`),
   nyaris instan, tidak ada jeda download
3. Ulangi cek poin A.4–A.7

**C. Callback & fungsi gameplay yang bergantung ke CEF**
1. Semua `cef_create_browser` / `cef_load_url` / `cef_destroy_browser`
   yang dipanggil `iternal_cef_phase*.inc` — test tiap fitur yang
   memicu salah satu dari itu (buka toko, buka ATM, dialog karakter,
   drone HUD, crafting, dll — lihat daftar di
   `gamemodes/SERVER/utils/iternal_cef_bridge.inc`)
2. `OnCefBrowserCreated` / status code — pastikan tidak ada yang balik
   status gagal di log server
3. Test spawn ulang (mati → respawn), ganti kendaraan, teleport antar
   interior — CEF yang nempel di HUD tidak boleh hilang/blank setelahnya
4. Test alt+tab / minimize-restore game saat CEF lagi kebuka — jangan
   sampai crash

**D. Edge case jaringan (ini yang paling sering kebawa ke production
   tanpa ketahuan kalau tidak ditest manual)**
1. Putuskan WiFi/koneksi **di tengah-tengah** proses download bundel
   pertama kali → harus gagal dengan rapi (log error, bukan crash), lalu
   waktu reconnect harus coba download ulang dari awal, bukan stuck
2. Simulasikan server **tidak** punya `ui.bin`/`ui_key.hex` sama sekali
   (hapus sementara / jangan jalankan `pack_ui_assets.py` dulu) → client
   harus tetap jalan seperti sistem lama (baca `cef/assets/*.html` kalau
   ada, atau browser kosong kalau tidak ada — intinya **tidak boleh
   crash**, ini jalur backward-compat)
3. Konek pakai `.asi` **versi lama** (sebelum patch ini) ke server yang
   **sudah** pakai `ui.bin` → harus tetap connect (dapat timeout fail-open
   60 detik, CEF-nya memang tidak akan tampil karena client lama tidak
   paham bundel baru, tapi game tidak boleh freeze/stuck)
4. Dua PC test konek ke **dua server CEF berbeda** bergantian → pastikan
   cache tidak saling tertukar (`cef/ui-cache/<ip1>/` vs
   `cef/ui-cache/<ip2>/` terpisah)

**E. Stabilitas umum / stress**
1. Biarkan player idle lama (30+ menit) dengan beberapa browser CEF
   terbuka — cek tidak ada memory leak signifikan (Task Manager → memory
   `gta_sa.exe` dari awal connect vs setelah 30 menit)
2. Buka-tutup browser CEF yang sama berkali-kali cepat (spam command yang
   trigger `cef_create_browser`/`cef_destroy_browser`) — tidak boleh
   crash/freeze
3. Beberapa player online bersamaan, request download bundel di waktu
   hampir sama (mis. restart server pas banyak yang reconnect) — pastikan
   server tidak keteteran/macet ngirim banyak `AssetChunk` sekaligus

## Catatan performa / kenapa ini aman dari freeze

- Proses berat (hash+tulis file+dekripsi bundel) cuma kejadian **sekali**
  per sesi — begitu `AssetChunk` terakhir diterima, bukan per-potongan.
  Untuk ukuran bundel kamu sekarang (~5-10 MB), AES-256-GCM dengan AES-NI
  hardware di CPU modern itu hitungan milidetik, bukan detik — jadi paling
  kerasa cuma hitch sangat singkat, bukan freeze.
- Dekripsi per-file saat CEF minta halaman (`assets_scheme.rs`) itu tiap
  file kecil (HTML belasan KB, icon puluhan-ratusan KB) — overhead-nya
  jauh lebih kecil dari baca file biasa dari disk.
- **Tapi**: kalau ke depan UI kamu tambah berat sampai puluhan-ratusan MB
  (banyak video/audio di CEF misalnya), proses tulis+dekripsi sekali-jalan
  itu bisa mulai kerasa nge-freeze beberapa ratus ms. Kalau itu kejadian,
  kabari saya — solusinya pindahkan proses itu ke thread terpisah
  (sekarang sengaja saya taruh sinkron di thread utama dulu, lebih
  sederhana & lebih kecil kemungkinan bug untuk ukuran file kamu saat
  ini).

## Catatan jujur soal batasan lain

- Ini menaikkan kesulitan dari "buka Notepad" jadi "harus
  reverse-engineer binary Rust" — bukan proteksi mutlak terhadap orang
  yang benar-benar niat (memory dump dsb).
- Belum dikerjakan: kunci DevTools paksa + minify/obfuscate JS sebelum
  di-pack. Bilang kalau mau saya lanjutkan.
- Download tidak resumable (putus = ulang dari nol) — cukup untuk ukuran
  sekarang, tapi kalau bundel jadi besar layak ditingkatkan.
- `Cargo.lock` perlu diregenerate (dependency baru `aes-gcm`, `sha2`).
  Kalau CI pakai `--locked`, jalankan `cargo update -p aes-gcm -p sha2`
  dulu sebelum push, atau hapus `Cargo.lock` dan biarkan CI regenerate.
- Saya **tidak** bisa compile binary apa pun di sesi ini (Windows maupun
  Linux, lihat "Status jujur" di atas) — semua "build .asi/.dll/.so baru"
  yang kamu minta akan keluar dari CI GitHub Actions kamu, bukan dari
  saya langsung.
