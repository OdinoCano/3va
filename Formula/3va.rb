class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.12.3"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.3/3va-v2.12.3-x86_64-apple-darwin.tar.gz"
      sha256 "51ec26d53425a56259afd3f7227afbd888d5db41fa617783403bd7b7e92cde96"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.3/3va-v2.12.3-aarch64-apple-darwin.tar.gz"
      sha256 "f9679697ac3efd722e2f8f6fb84ffe86cb83f4de47e4c9d0e3529529e5dd5a2b"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.3/3va-v2.12.3-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "a805d04746f44ba926dd8d0b308102586eafa6fe53c211c4ed1b9f19656b28f0"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.3/3va-v2.12.3-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "aeddcb4c988335156e37e4e76c12716e79bd1e2dc8dee56014e0f15adf27d9ea"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
