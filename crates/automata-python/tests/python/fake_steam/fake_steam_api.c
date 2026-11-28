/* A stand-in for Valve's steam_api library, for tests: the flat API
 * functions automata.steam calls, with no Steam behind them.
 *
 *   cc -shared -fPIC -o libsteam_api.so fake_steam_api.c [-DOLD_SDK]
 *
 * Environment:
 *   FAKE_STEAM_LOG           file every call is appended to, one per line
 *   FAKE_STEAM_FAIL          make SteamAPI_InitFlat fail with this message
 *   FAKE_STEAM_ACHIEVEMENTS  the game's achievements: "ACH_A,ACH_B"
 *   FAKE_STEAM_STATS         its stats: "kills:i,distance:f"
 *   FAKE_STEAM_RESTART       SteamAPI_RestartAppIfNecessary returns true
 *
 * -DOLD_SDK builds an older library: no SteamAPI_InitFlat (SteamAPI_Init
 * instead), and stats arrive only after RequestCurrentStats, as a
 * UserStatsReceived_t callback on the next frame.
 *
 * FakeSteam_SetOverlay(open) queues a GameOverlayActivated_t, as the
 * player pressing Shift+Tab would; opening a dialog does too. */

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#define API __declspec(dllexport)
#pragma pack(push, 8)
#else
#define API __attribute__((visibility("default")))
#pragma pack(push, 4)
#endif
typedef struct {
    int32_t user;
    int32_t callback;
    uint8_t *param;
    int32_t size;
} CallbackMsg;
typedef struct {
    uint8_t active;
    bool user_initiated;
    uint32_t app_id;
    uint32_t pid;
} OverlayActivated;
typedef struct {
    uint64_t game_id;
    int32_t result;
    uint64_t user;
} StatsReceived;
#pragma pack(pop)

#define MAX 32
static char ach_names[MAX][64];
static bool ach_done[MAX];
static int n_ach;
static char stat_names[MAX][64];
static char stat_kind[MAX];
static double stat_value[MAX];
static int n_stats;
static bool loaded, running, stats_ready = true, stats_queued;
static int stores;
static uint32_t app_id;

static uint8_t queue[16][32];
static int queue_kind[16], queue_size[16], q_head, q_tail;
static uint8_t current[32];

static int self_token = 1;  /* every interface is this address */

static void logf_(const char *fmt, const char *a, double b) {
    const char *path = getenv("FAKE_STEAM_LOG");
    if (!path) return;
    FILE *f = fopen(path, "a");
    if (!f) return;
    fprintf(f, fmt, a ? a : "", b);
    fputc('\n', f);
    fclose(f);
}

static void push(int kind, const void *data, int size) {
    if ((q_tail + 1) % 16 == q_head) return;
    memcpy(queue[q_tail], data, size);
    queue_kind[q_tail] = kind;
    queue_size[q_tail] = size;
    q_tail = (q_tail + 1) % 16;
}

static void load(void) {
    if (loaded) return;
    loaded = true;
    char buf[512];
    const char *a = getenv("FAKE_STEAM_ACHIEVEMENTS");
    if (a) {
        strncpy(buf, a, sizeof buf - 1);
        buf[sizeof buf - 1] = 0;
        for (char *t = strtok(buf, ","); t && n_ach < MAX; t = strtok(NULL, ","))
            strncpy(ach_names[n_ach++], t, 63);
    }
    const char *s = getenv("FAKE_STEAM_STATS");
    if (s) {
        strncpy(buf, s, sizeof buf - 1);
        buf[sizeof buf - 1] = 0;
        for (char *t = strtok(buf, ","); t && n_stats < MAX; t = strtok(NULL, ",")) {
            char *colon = strchr(t, ':');
            stat_kind[n_stats] = colon ? colon[1] : 'i';
            if (colon) *colon = 0;
            strncpy(stat_names[n_stats++], t, 63);
        }
    }
    const char *id = getenv("SteamAppId");
    app_id = id ? (uint32_t)atoi(id) : 0;
}

static int find(char names[][64], int n, const char *name) {
    for (int i = 0; i < n; i++)
        if (strcmp(names[i], name) == 0) return i;
    return -1;
}

#ifndef OLD_SDK
API int SteamAPI_InitFlat(char *err) {
    load();
    const char *fail = getenv("FAKE_STEAM_FAIL");
    logf_("InitFlat app=%s", getenv("SteamAppId"), 0);
    if (fail) {
        if (err) snprintf(err, 1024, "%s", fail);
        return 2;
    }
    running = true;
    return 0;
}
#else
API bool SteamAPI_Init(void) {
    load();
    logf_("Init app=%s", getenv("SteamAppId"), 0);
    if (getenv("FAKE_STEAM_FAIL")) return false;
    running = true;
    stats_ready = false;
    return true;
}
API bool SteamAPI_ISteamUserStats_RequestCurrentStats(void *self) {
    (void)self;
    logf_("RequestCurrentStats", NULL, 0);
    stats_queued = true;
    return true;
}
#endif

API void SteamAPI_Shutdown(void) {
    logf_("Shutdown", NULL, 0);
    running = false;
}
API bool SteamAPI_RestartAppIfNecessary(uint32_t id) {
    logf_("RestartAppIfNecessary %s%.0f", "", id);
    return getenv("FAKE_STEAM_RESTART") != NULL;
}
API void SteamAPI_RunCallbacks(void) {}
API int32_t SteamAPI_GetHSteamPipe(void) { return 7; }
API void SteamAPI_ManualDispatch_Init(void) { logf_("ManualDispatch_Init", NULL, 0); }
API void SteamAPI_ManualDispatch_RunFrame(int32_t pipe) {
    (void)pipe;
    if (stats_queued) {
        stats_queued = false;
        stats_ready = true;
        StatsReceived r = {app_id, 1, 76561197960287930ULL};
        push(1101, &r, sizeof r);
    }
}
API bool SteamAPI_ManualDispatch_GetNextCallback(int32_t pipe, CallbackMsg *msg) {
    if (pipe != 7 || q_head == q_tail) return false;
    memcpy(current, queue[q_head], sizeof current);
    msg->user = 1;
    msg->callback = queue_kind[q_head];
    msg->param = current;
    msg->size = queue_size[q_head];
    q_head = (q_head + 1) % 16;
    return true;
}
API void SteamAPI_ManualDispatch_FreeLastCallback(int32_t pipe) { (void)pipe; }

API void *SteamAPI_SteamUserStats_v013(void) { return running ? &self_token : NULL; }
API void *SteamAPI_SteamFriends_v018(void) { return running ? &self_token : NULL; }
API void *SteamAPI_SteamUtils_v011(void) { return running ? &self_token : NULL; }
API void *SteamAPI_SteamApps_v009(void) { return running ? &self_token : NULL; }
API void *SteamAPI_SteamRemoteStorage_v016(void) { return running ? &self_token : NULL; }

API bool SteamAPI_ISteamUserStats_SetAchievement(void *self, const char *name) {
    (void)self;
    int i = find(ach_names, n_ach, name);
    if (i < 0 || !stats_ready) return false;
    ach_done[i] = true;
    logf_("SetAchievement %s", name, 0);
    return true;
}
API bool SteamAPI_ISteamUserStats_GetAchievement(void *self, const char *name, bool *got) {
    (void)self;
    int i = find(ach_names, n_ach, name);
    if (i < 0 || !stats_ready) return false;
    *got = ach_done[i];
    return true;
}
API bool SteamAPI_ISteamUserStats_ClearAchievement(void *self, const char *name) {
    (void)self;
    int i = find(ach_names, n_ach, name);
    if (i < 0) return false;
    ach_done[i] = false;
    logf_("ClearAchievement %s", name, 0);
    return true;
}
API bool SteamAPI_ISteamUserStats_IndicateAchievementProgress(void *self, const char *name,
                                                              uint32_t cur, uint32_t max) {
    (void)self;
    (void)max;
    logf_("IndicateAchievementProgress %s %.0f", name, cur);
    return find(ach_names, n_ach, name) >= 0;
}
API uint32_t SteamAPI_ISteamUserStats_GetNumAchievements(void *self) {
    (void)self;
    return (uint32_t)n_ach;
}
API const char *SteamAPI_ISteamUserStats_GetAchievementName(void *self, uint32_t i) {
    (void)self;
    return i < (uint32_t)n_ach ? ach_names[i] : NULL;
}
API bool SteamAPI_ISteamUserStats_GetStatInt32(void *self, const char *name, int32_t *out) {
    (void)self;
    int i = find(stat_names, n_stats, name);
    if (i < 0 || stat_kind[i] != 'i' || !stats_ready) return false;
    *out = (int32_t)stat_value[i];
    return true;
}
API bool SteamAPI_ISteamUserStats_GetStatFloat(void *self, const char *name, float *out) {
    (void)self;
    int i = find(stat_names, n_stats, name);
    if (i < 0 || stat_kind[i] != 'f' || !stats_ready) return false;
    *out = (float)stat_value[i];
    return true;
}
API bool SteamAPI_ISteamUserStats_SetStatInt32(void *self, const char *name, int32_t v) {
    (void)self;
    int i = find(stat_names, n_stats, name);
    if (i < 0 || stat_kind[i] != 'i' || !stats_ready) return false;
    stat_value[i] = v;
    logf_("SetStatInt32 %s %.0f", name, v);
    return true;
}
API bool SteamAPI_ISteamUserStats_SetStatFloat(void *self, const char *name, float v) {
    (void)self;
    int i = find(stat_names, n_stats, name);
    if (i < 0 || stat_kind[i] != 'f' || !stats_ready) return false;
    stat_value[i] = v;
    logf_("SetStatFloat %s %g", name, v);
    return true;
}
API bool SteamAPI_ISteamUserStats_StoreStats(void *self) {
    (void)self;
    logf_("StoreStats %s%.0f", "", ++stores);
    return true;
}
API bool SteamAPI_ISteamUserStats_ResetAllStats(void *self, bool achievements) {
    (void)self;
    for (int i = 0; i < n_stats; i++) stat_value[i] = 0;
    if (achievements)
        for (int i = 0; i < n_ach; i++) ach_done[i] = false;
    logf_("ResetAllStats %s%.0f", "", achievements);
    return true;
}

API const char *SteamAPI_ISteamFriends_GetPersonaName(void *self) {
    (void)self;
    return "Fake Player";
}
API void FakeSteam_SetOverlay(int open) {
    OverlayActivated o = {(uint8_t)(open ? 1 : 0), true, app_id, 0};
    push(331, &o, sizeof o);
}
API void SteamAPI_ISteamFriends_ActivateGameOverlay(void *self, const char *dialog) {
    (void)self;
    logf_("ActivateGameOverlay %s", dialog, 0);
    FakeSteam_SetOverlay(1);
}
API void SteamAPI_ISteamFriends_ActivateGameOverlayToStore(void *self, uint32_t id, int flag) {
    (void)self;
    (void)flag;
    logf_("ActivateGameOverlayToStore %s%.0f", "", id);
}
API void SteamAPI_ISteamFriends_ActivateGameOverlayToWebPage(void *self, const char *url, int mode) {
    (void)self;
    (void)mode;
    logf_("ActivateGameOverlayToWebPage %s", url, 0);
}
API uint32_t SteamAPI_ISteamUtils_GetAppID(void *self) {
    (void)self;
    return app_id;
}
API bool SteamAPI_ISteamUtils_IsOverlayEnabled(void *self) {
    (void)self;
    return true;
}
API bool SteamAPI_ISteamUtils_IsSteamInBigPictureMode(void *self) {
    (void)self;
    return false;
}
API const char *SteamAPI_ISteamApps_GetCurrentGameLanguage(void *self) {
    (void)self;
    return "english";
}
/* Steam Cloud: on for the account; on for the game when FAKE_STEAM_CLOUD is set. */
API bool SteamAPI_ISteamRemoteStorage_IsCloudEnabledForAccount(void *self) {
    (void)self;
    return true;
}
API bool SteamAPI_ISteamRemoteStorage_IsCloudEnabledForApp(void *self) {
    (void)self;
    return getenv("FAKE_STEAM_CLOUD") != NULL;
}
