class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.1.3"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.3/3va-v2.1.3-x86_64-apple-darwin.tar.gz"
      sha256 "fe19a439f065c393c3ab8be59d3ffe326511eb765a75c4ace261982bca479537"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.3/3va-v2.1.3-aarch64-apple-darwin.tar.gz"
      sha256 "dc8214db094b0676811bf1e27550fbc6ca3bd797ca1cf4afbf5d8080332f14bb"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.3/3va-v2.1.3-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "16115c59ac641f2aa7666724f9b4a28194e1efe31fa1902b6e2a714abc44bb50"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.3/3va-v2.1.3-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "8c0a9ba5fd1987a73fcb2cd0fd0e647d7f13512cea1ad77a22a0d47870f54d7b"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
