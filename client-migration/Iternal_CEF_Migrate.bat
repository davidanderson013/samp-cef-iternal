@echo off
setlocal enabledelayedexpansion
title Iternal RP - Migrasi CEF ke sistem UI terenkripsi
color 0B

echo ============================================================
echo   Migrasi plugin CEF Iternal RP -^> sistem UI terenkripsi
echo ============================================================
echo.
echo Script ini akan:
echo   1. Menutup GTA SA / cef-renderer kalau masih jalan
echo   2. Memindahkan cef\assets lama (HTML polos) ke backup
echo   3. Menghapus cache CEF lama (cache/GPUCache/user_data)
echo   4. Menimpa cef.asi, cef\cef-client.dll, cef\cef-renderer.exe,
echo      dkk dengan build baru dari folder "cef_update"
echo.

cd /d "%~dp0"

if not exist "cef_update\" (
    echo [GAGAL] Folder "cef_update" tidak ditemukan di sini.
    echo.
    echo Taruh dulu hasil build terbaru dari GitHub Actions kamu
    echo ^(artifact cef.zip, atau minimal cef.asi + isi folder cef\^)
    echo ke dalam folder bernama "cef_update" di sebelah file .bat ini,
    echo lalu jalankan lagi.
    echo.
    pause
    exit /b 1
)

echo [1/5] Menutup proses yang masih jalan...
taskkill /f /im gta_sa.exe   >nul 2>&1
taskkill /f /im cef-renderer.exe >nul 2>&1
timeout /t 2 /nobreak >nul

echo [2/5] Membackup cef\assets lama ^(kalau ada^)...
if exist "cef\assets\" (
    if exist "cef_assets_backup_old\" (
        rmdir /s /q "cef_assets_backup_old"
    )
    move "cef\assets" "cef_assets_backup_old" >nul
    echo       -^> dipindah ke "cef_assets_backup_old"
    echo       ^(boleh dihapus manual nanti setelah yakin UI baru jalan normal^)
) else (
    echo       -^> tidak ada cef\assets lama, dilewati
)

echo [3/5] Menghapus cache CEF lama...
if exist "cef\cache"      rmdir /s /q "cef\cache"
if exist "cef\GPUCache"   rmdir /s /q "cef\GPUCache"
if exist "cef\user_data"  rmdir /s /q "cef\user_data"
if exist "cef\ui-cache"   rmdir /s /q "cef\ui-cache"

echo [4/5] Menimpa plugin lama dengan build baru dari cef_update\...
xcopy /e /y /i "cef_update\*" "." >nul
if errorlevel 1 (
    echo [GAGAL] xcopy gagal menyalin file. Cek isi folder cef_update.
    pause
    exit /b 1
)

echo [5/5] Bersih-bersih folder cef_update...
rmdir /s /q "cef_update"

echo.
echo ============================================================
echo  Selesai. Buka SA-MP dan konek ke server seperti biasa.
echo  UI akan otomatis ter-download terenkripsi dari server saat
echo  pertama kali connect (butuh waktu sebentar, lihat progress
echo  di cef_client.log kalau mau pastikan).
echo ============================================================
echo.
pause
