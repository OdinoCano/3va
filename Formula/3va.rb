class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.0.2"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-x86_64-apple-darwin.tar.gz"
      sha256 "44b4b795b239015b49bf18da1d653fde8e01048fa05417dfe06cd7f09599900a"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-aarch64-apple-darwin.tar.gz"
      sha256 "f9af97a78f33b917b6675256c947fae58c20d669ad6648235866580b61ee59e2"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "fd0773b6a9f2bf8e3340b35e8a5da8a76303700de8e8c39f039cd1f1439735b0"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "715d95dbd1dcf3ed5bed32db0000f74aa510590e955efd7eaaf75fcfbba03e56"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
