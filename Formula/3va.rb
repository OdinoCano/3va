class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.11.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.11.0/3va-v2.11.0-x86_64-apple-darwin.tar.gz"
      sha256 "befd2435f1fe842f9d5354a3cd592d26055d8fd81c8a6fc4b2cd5731d376df1e"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.11.0/3va-v2.11.0-aarch64-apple-darwin.tar.gz"
      sha256 "53c85d6540dfa044fadfcc525e0fb6ca5fa82f3019f469f0e8d94a13394365d4"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.11.0/3va-v2.11.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "1d6e58c3d75ff8f8de0a393f27cedf09b12d0696ea5fe46208b795a5ffaa4433"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.11.0/3va-v2.11.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "a9c9d703059731d674fb45454e5954ad27e3d663b7f1ddb5e21721efdb78cc4d"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
