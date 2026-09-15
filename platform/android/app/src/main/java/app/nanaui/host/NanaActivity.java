package app.nanaui.host;

import com.google.androidgamesdk.GameActivity;

public class NanaActivity extends GameActivity {
    static {
        System.loadLibrary("nana_android_host");
    }
}
