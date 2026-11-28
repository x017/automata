package org.automata;

import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;

import org.kivy.android.PythonActivity;

/**
 * The game's activity: python-for-android's, changed so the game survives
 * Android re-creating the activity.
 *
 * Android re-creates an activity for configuration changes the manifest
 * doesn't list: the first rotation to landscape, bold text, a new system
 * theme after a wallpaper change (resource overlays). SDL handles that by
 * quitting the game's main() and running it again on the new activity;
 * python-for-android's main() runs Python once and then ends the process,
 * so the game would close.
 *
 * Instead the game keeps running on SDL's thread through the re-creation,
 * as if it went to the background and came back: the old activity lets go
 * without telling SDL to quit, and the new one adopts the running thread
 * and gives it a new surface (the engine already handles that through
 * SDL's background/foreground events).
 *
 * Re-creation can also come while the app is still unpacking, before SDL's
 * library is loaded. Then both activities' unpack tasks end by calling
 * finishLoad on the newest activity, and SDL's second-activity check runs
 * before anything could allow it. So finishLoad loads the libraries and
 * allows re-creation itself first, and runs only once per activity.
 */
public class AutomataActivity extends PythonActivity {
    private static final String TAG = "automata";

    private static final String ALLOW_RECREATE = "SDL_ANDROID_ALLOW_RECREATE_ACTIVITY";

    /** SDL's thread while a re-created activity takes it over. */
    private static Thread sKeptThread = null;

    /** Whether this activity's finishLoad has run. */
    private boolean mLoaded = false;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // super.onCreate cleared SDL's record of its thread.
        adoptKeptThread();
        // SDL reads hints from its own copy of the environment, which only
        // nativeSetenv changes, and only once SDL's library is loaded (by
        // the first activity's finishLoad). Re-creation happens later in
        // the same process, when the copy has the setting, and SDL checks
        // it in finishLoad.
        try {
            nativeSetenv(ALLOW_RECREATE, "1");
        } catch (UnsatisfiedLinkError e) {
            Log.i(TAG, "re-creation is allowed once the libraries load");
        }
    }

    /**
     * Loads SDL's library (the files are unpacked by now) and sets the hint
     * that lets SDL accept a re-created activity, before SDL's own
     * finishLoad checks it. Loading again later is harmless.
     */
    private void allowRecreation() {
        try {
            loadLibraries();
            nativeSetenv(ALLOW_RECREATE, "1");
            // The player unpacks a game built into the APK from here.
            nativeSetenv("AUTOMATA_APK", getApplicationInfo().sourceDir);
        } catch (UnsatisfiedLinkError | RuntimeException e) {
            // SDL's finishLoad reports a library that doesn't load.
            Log.w(TAG, "couldn't allow activity re-creation: " + e);
        }
    }

    @Override
    protected void finishLoad() {
        if (mLoaded) {
            // The unpack task of an activity re-created before loading
            // finished: this activity already loaded (and may be running
            // the game), and loading again would reset SDL under it.
            Log.i(TAG, "already loaded; skipping a second finishLoad");
            if (mAppConfirmedActive) {
                // The caller shows the loading screen again afterwards.
                new Handler(Looper.getMainLooper()).postDelayed(new Runnable() {
                    @Override
                    public void run() {
                        removeLoadingScreen();
                    }
                }, 500);
            }
            return;
        }
        mLoaded = true;
        allowRecreation();
        super.finishLoad();
        // finishLoad cleared SDL's record of its thread again.
        if (adoptKeptThread()) {
            sKeptThread = null;
            Log.i(TAG, "the new activity took over the running game");
            // main() doesn't start again, so nothing confirms the game is
            // running and the loading screen (which python-for-android
            // shows again after finishLoad) would stay. Remove it then.
            new Handler(Looper.getMainLooper()).post(new Runnable() {
                @Override
                public void run() {
                    appConfirmedActive();
                    removeLoadingScreen();
                }
            });
        }
    }

    @Override
    protected void onDestroy() {
        if (isChangingConfigurations()) {
            Thread game = mSDLThread;
            boolean running = game != null && game.isAlive() && !mBrokenLibraries;
            Log.i(TAG, "activity re-created for configuration changes 0x"
                    + Integer.toHexString(getChangingConfigurations())
                    + (running ? "; the game keeps running" : ""));
            if (running) {
                sKeptThread = game;
                // SDL's onDestroy would tell the game to quit and free what
                // its thread waits on. With the libraries marked unusable it
                // only lets the activity go: the state a new activity is in
                // anyway until its finishLoad, which clears the mark.
                mBrokenLibraries = true;
            }
        }
        super.onDestroy();
    }

    /** Gives SDL back the thread kept through a re-creation, if any. */
    private static boolean adoptKeptThread() {
        Thread kept = sKeptThread;
        if (kept == null) {
            return false;
        }
        if (!kept.isAlive()) {
            sKeptThread = null;
            return false;
        }
        mSDLThread = kept;
        return true;
    }
}
