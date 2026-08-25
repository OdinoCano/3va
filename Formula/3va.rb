class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.6.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-x86_64-apple-darwin.tar.gz"
      sha256 "e44afc86b29ff07e659277dc74de07d6977753d4f3b723fa776b078ffe922fea"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-aarch64-apple-darwin.tar.gz"
      sha256 "b755331be6f291b831e04973adfff28c9bb79d1403f228445110a389f18d7d23"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "02e705eec73aa2ac3a905b9fa68ce436593abddaf2a380e776cc190187118f27"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "6f62ca90ae23a86287d59d95ec7f29417d811f77624ec759b1f46c4b0f1a459d"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
