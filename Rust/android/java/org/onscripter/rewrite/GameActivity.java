package org.onscripter.rewrite;

import android.content.Intent;
import android.os.Bundle;
import org.libsdl.app.SDLActivity;
import java.io.IOException;

public final class GameActivity extends SDLActivity {
    private GameDocuments documents;
    private String assetError = "Unable to access the selected folder.";
    @Override protected String[] getLibraries() { return new String[] {"SDL3", "onscripter_android"}; }
    @Override protected void onCreate(Bundle saved) {
        documents = new GameDocuments(this, getSharedPreferences("folders", MODE_PRIVATE));
        super.onCreate(saved);
    }
    // Called on SDL's engine thread. Ownership of a returned descriptor passes to Rust.
    public int openAssetFd(String name) {
        try { return documents.open(name); }
        catch (IOException | RuntimeException error) {
            assetError = error.getMessage() == null ? "Unable to access the selected folder; choose it again." : error.getMessage();
            return -2;
        }
    }
    public String getAssetError() { return assetError; }
    public void showEngineError(String message) {
        runOnUiThread(() -> {
            startActivity(new Intent(this,SetupActivity.class).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP)
                .putExtra("engineError",message));
            finish();
        });
    }
}
