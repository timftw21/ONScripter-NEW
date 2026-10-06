package org.onscripter.rewrite;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Bundle;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;

public final class SetupActivity extends Activity {
    private static final int GAME = 1, OVERLAY = 2;
    private SharedPreferences preferences;
    private TextView status;

    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        preferences = getSharedPreferences("folders", MODE_PRIVATE);
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        int padding = (int)(24 * getResources().getDisplayMetrics().density);
        layout.setPadding(padding,padding,padding,padding);
        status = new TextView(this);
        layout.addView(status);
        button(layout,"Choose game folder", () -> choose(GAME));
        button(layout,"Choose modified-asset folder", () -> choose(OVERLAY));
        button(layout,"Clear modified-asset folder", () -> {
            String old = preferences.getString("overlay", "");
            preferences.edit().remove("overlay").apply();
            releaseUnused(old,"overlay"); update();
        });
        button(layout,"Play", () -> {
            if (preferences.contains("game")) startActivity(new Intent(this, GameActivity.class));
            else choose(GAME);
        });
        setContentView(layout);
        update();
        showError(getIntent());
    }
    @Override protected void onNewIntent(Intent intent) { super.onNewIntent(intent); setIntent(intent); showError(intent); }
    private void showError(Intent intent) {
        String error = intent.getStringExtra("engineError");
        if (error != null) { intent.removeExtra("engineError"); new AlertDialog.Builder(this).setTitle("Unable to play").setMessage(error).setPositiveButton("Close",null).show(); }
    }
    private void button(LinearLayout layout, String label, Runnable action) {
        Button button = new Button(this); button.setText(label);
        button.setOnClickListener(view -> action.run()); layout.addView(button);
    }
    private void choose(int request) {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION | Intent.FLAG_GRANT_PREFIX_URI_PERMISSION);
        startActivityForResult(intent, request);
    }
    @Override public void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request,result,data);
        if (result != RESULT_OK || data == null || data.getData() == null) return;
        Uri uri = data.getData();
        try { getContentResolver().takePersistableUriPermission(uri, data.getFlags() & Intent.FLAG_GRANT_READ_URI_PERMISSION); }
        catch (SecurityException error) { new AlertDialog.Builder(this).setTitle("Unable to access folder").setMessage(error.getMessage()).setPositiveButton("Close",null).show(); return; }
        String key = request == GAME ? "game" : "overlay";
        String old = preferences.getString(key, "");
        preferences.edit().putString(key,uri.toString()).apply();
        if (!old.equals(uri.toString())) releaseUnused(old,key);
        update();
    }
    private void releaseUnused(String uri, String changed) {
        if (!uri.isEmpty() && !uri.equals(preferences.getString(changed.equals("game") ? "overlay" : "game", ""))) {
            try { getContentResolver().releasePersistableUriPermission(Uri.parse(uri),Intent.FLAG_GRANT_READ_URI_PERMISSION); }
            catch (SecurityException revoked) { /* The grant may already have been revoked in Settings. */ }
        }
    }
    private void update() {
        status.setText("Game: " + preferences.getString("game", "Choose a folder") + "\nModified assets: " + preferences.getString("overlay", "None"));
    }
}
