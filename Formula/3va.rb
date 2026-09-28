class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.10.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.10.0/3va-v2.10.0-x86_64-apple-darwin.tar.gz"
      sha256 "f44f9c77227355920d13c5bab99a595c1f2ceef22c38f1422463fb2731392fd3"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.10.0/3va-v2.10.0-aarch64-apple-darwin.tar.gz"
      sha256 "905bff0a93646bb96e5a98b7f52e0753a94e92c60b480a125cd0a4d41b058528"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.10.0/3va-v2.10.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "f680943400cbd65151d3116c7c40c694d659c3391199705514329d4c9b2f077b"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.10.0/3va-v2.10.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "9bcd33f334d5788cad53820871e5315aadf501519ab8e846a7013944efd63b91"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
