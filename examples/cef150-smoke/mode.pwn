#include <open.mp>
#include <cef>

#define SMOKE_BROWSER_ID (150)
#define SMOKE_BROWSER_URL "cef150-smoke/index.html"
#define OBJECT_BROWSER_ID (151)
#define OBJECT_BROWSER_URL "cef150-smoke/object.html"

new SmokeObject = INVALID_OBJECT_ID;
new Text:FilterProbe = Text:INVALID_TEXT_DRAW;
new bool:FilterProbeVisible[MAX_PLAYERS];

forward OnSmokeClient(playerid, const arguments[]);

main() {}

public OnGameModeInit()
{
    SetGameModeText("CEF 150 smoke test");
    AddPlayerClass(0, 1958.3783, 1343.1572, 15.3746, 269.1425, WEAPON_FIST, 0, WEAPON_FIST, 0, WEAPON_FIST, 0);
    cef_subscribe("smoke:client", "OnSmokeClient");
    SmokeObject = CreateObject(19371, 0.0, 0.0, 3.0, 0.0, 0.0, 90.0);
    SetObjectMaterial(SmokeObject, 0, 19341, "egg_texts", "easter_egg01", 0xFFFFFFFF);

    FilterProbe = TextDrawCreate(320.0, 120.0, "TEXTDRAW FILTER PROBE");
    TextDrawAlignment(FilterProbe, TEXT_DRAW_ALIGN_CENTER);
    TextDrawLetterSize(FilterProbe, 0.45, 1.8);
    TextDrawTextSize(FilterProbe, 620.0, 80.0);
    TextDrawColour(FilterProbe, 0xFFFFFFFF);
    TextDrawUseBox(FilterProbe, true);
    TextDrawBoxColour(FilterProbe, 0xCC111111);
    TextDrawSetSelectable(FilterProbe, true);

    printf("[cef-smoke] object created id=%d", SmokeObject);
    print("[cef-smoke] gamemode initialized");
    return 1;
}

public OnCefInitialize(player_id, success)
{
    printf("[cef-smoke] OnCefInitialize player=%d success=%d", player_id, success);
    if (success)
    {
        cef_create_browser(player_id, SMOKE_BROWSER_ID, SMOKE_BROWSER_URL, false, true);
        cef_create_ext_browser(player_id, OBJECT_BROWSER_ID, "easter_egg01", OBJECT_BROWSER_URL, 2);

        if (SmokeObject != INVALID_OBJECT_ID)
        {
            new appended = cef_append_to_object(player_id, OBJECT_BROWSER_ID, SmokeObject);
            printf("[cef-smoke] object append player=%d browser=%d object=%d result=%d", player_id, OBJECT_BROWSER_ID, SmokeObject, appended);
        }
    }
    return 1;
}

public OnCefBrowserCreated(player_id, browser_id, status_code)
{
    printf("[cef-smoke] OnCefBrowserCreated player=%d browser=%d status=%d", player_id, browser_id, status_code);

    if (browser_id == OBJECT_BROWSER_ID && status_code == 200)
    {
        print("[cef-smoke] object-texture browser loaded; overlay remains visible for the main smoke checks");
    }
    return 1;
}

public OnSmokeClient(playerid, const arguments[])
{
    printf("[cef-smoke] JS IPC player=%d args=%s", playerid, arguments);
    new sent = cef_emit_event(playerid, "smoke:server", CEFSTR("pong"));
    printf("[cef-smoke] server IPC sent=%d", sent);

    if (!strcmp(arguments, "button", true))
    {
        cef_focus_browser(playerid, SMOKE_BROWSER_ID, false);
        cef_hide_browser(playerid, SMOKE_BROWSER_ID, true);
        print("[cef-smoke] overlay hidden; object-texture view active");
    }

    return 1;
}

public OnPlayerRequestClass(playerid, classid)
{
    SetSpawnInfo(playerid, NO_TEAM, 0, 1958.3783, 1343.1572, 15.3746, 269.1425, WEAPON_FIST, 0, WEAPON_FIST, 0, WEAPON_FIST, 0);
    SpawnPlayer(playerid);
    return 1;
}

public OnPlayerSpawn(playerid)
{
    SetPlayerPos(playerid, 0.0, -8.0, 3.0);
    SetPlayerCameraPos(playerid, 0.0, -8.0, 3.0);
    SetPlayerCameraLookAt(playerid, 0.0, 0.0, 3.0);
    return 1;
}

public OnPlayerCommandText(playerid, cmdtext[])
{
    if (!strcmp(cmdtext, "/textdraw", true))
    {
        FilterProbeVisible[playerid] = !FilterProbeVisible[playerid];

        if (FilterProbeVisible[playerid])
        {
            TextDrawShowForPlayer(playerid, FilterProbe);
            SelectTextDraw(playerid, 0x66FFFFFF);
        }
        else
        {
            CancelSelectTextDraw(playerid);
            TextDrawHideForPlayer(playerid, FilterProbe);
        }

        return 1;
    }

    return 0;
}
