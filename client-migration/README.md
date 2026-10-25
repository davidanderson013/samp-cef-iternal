# Cara pakai Iternal_CEF_Migrate.bat

Ini untuk PC **player** (atau PC test kamu) — migrasi dari sistem CEF lama
(file `.html` polos di `cef\assets\`) ke sistem baru (bundel `ui.bin`
terenkripsi, download otomatis dari server).

## Langkah

1. Build dulu lewat GitHub Actions kamu (workflow `make-package` yang
   sudah ada di repo `davidanderson013/samp-cef`), setelah source patch
   dari `CEF_UI_PROTECTION.md` sudah kamu push.
2. Download artifact `cef.zip` dari run itu.
3. Di PC player/test, buka folder GTA SA (tempat `gta_sa.exe` berada).
4. Buat folder baru bernama **`cef_update`** persis di situ.
5. Ekstrak isi `cef.zip` ke dalam folder `cef_update` itu — hasil akhirnya
   harus ada `cef_update\cef.asi`, `cef_update\cef\cef-client.dll`,
   `cef_update\cef\cef-renderer.exe`, dst.
6. Taruh `Iternal_CEF_Migrate.bat` ini di folder GTA SA (sejajar
   `gta_sa.exe`, sejajar folder `cef_update` yang baru dibuat).
7. Jalankan `Iternal_CEF_Migrate.bat`.

Script akan otomatis: tutup proses GTA/CEF yang nyangkut, backup
`cef\assets` lama (bukan dihapus permanen — dipindah ke
`cef_assets_backup_old`, aman kalau ternyata perlu rollback), bersihkan
cache CEF lama, timpa semua file dengan isi `cef_update\`, lalu hapus
folder `cef_update` yang sudah kosong.

Setelah itu tinggal buka SA-MP dan konek ke server seperti biasa — UI akan
otomatis ke-download dari server (lihat `CEF_UI_PROTECTION.md` untuk alur
lengkapnya dan cara setup di sisi server).

## Kalau mau rollback

Selama `cef_assets_backup_old` belum dihapus manual, tinggal:
1. Hapus `cef\assets` (versi baru, kalau ada)
2. Rename `cef_assets_backup_old` jadi `cef\assets`
3. Pasang lagi `cef.asi`/`cef-client.dll`/`cef-renderer.exe` versi lama

## Catatan

Script ini sengaja **tidak** menyentuh folder `cef\ui-cache` kalau kamu
jalankan ulang setelah migrasi pertama berhasil (dihapus hanya sekali saat
migrasi dari sistem lama) — biar player tidak perlu download ulang tiap
kali ada update kecil di luar UI bundle.
