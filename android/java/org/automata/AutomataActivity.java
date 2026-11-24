package org.automata;

import android.os.Bundle;
import android.util.Log;

import org.kivy.android.PythonActivity;

/**
 * The game's activity: python-for-android's, with two changes.
 *
 * Android may re-create the activity before the game starts (a
 * configuration change the manifest doesn't list). SDL treats any
 * re-creation as fatal unless allowed, and quits the app silently. The
 * engine keeps all its state on SDL's thread and gets the new surface
 * through SDL's background/foreground events, so re-creation is allowed.
 */
public class AutomataActivity extends PythonActivity {
    private static final String TAG = "automata";

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        try {
            // SDL reads its hints from the environment too.
            android.system.Os.setenv("SDL_ANDROID_ALLOW_RECREATE_ACTIVITY", "1", true);
        } catch (Exception e) {
            Log.w(TAG, "couldn't allow activity re-creation: " + e);
        }
        super.onCreate(savedInstanceState);
    }

    @Override
    protected void onDestroy() {
        if (isChangingConfigurations()) {
            Log.i(TAG, "activity re-created for configuration changes 0x"
                    + Integer.toHexString(getChangingConfigurations()));
        }
        super.onDestroy();
    }
}
