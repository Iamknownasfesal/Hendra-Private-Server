package kabam.rotmg.stage3D.proxies
{
import flash.display3D.Context3D;
import flash.geom.Matrix3D;

public class Context3DProxy
{


    private var context3D:Context3D;

    public function Context3DProxy(param1:Context3D)
    {
        super();
        this.context3D = param1;
    }

    public function GetContext3D() : Context3D
    {
        return this.context3D;
    }

    public function configureBackBuffer(param1:int, param2:int, param3:int, param4:Boolean = true) : void
    {
        this.context3D.configureBackBuffer(param1,param2,param3,param4);
    }

    public function createProgram() : Program3DProxy
    {
        return new Program3DProxy(this.context3D.createProgram());
    }

    public function clear() : void
    {
        this.context3D.clear(0.05,0.05,0.05);
    }

    public function present() : void
    {
        this.context3D.present();
    }

    public function createIndexBuffer(param1:int) : IndexBuffer3DProxy
    {
        return new IndexBuffer3DProxy(this.context3D.createIndexBuffer(param1));
    }

    public function createVertexBuffer(param1:int, param2:int) : VertexBuffer3DProxy
    {
        return new VertexBuffer3DProxy(this.context3D.createVertexBuffer(param1,param2));
    }

    public function setVertexBufferAt(param1:int, param2:VertexBuffer3DProxy, param3:int, param4:String = "float4") : void
    {
        this.context3D.setVertexBufferAt(param1,param2.getVertexBuffer3D(),param3,param4);
    }

    public function setProgramConstantsFromMatrix(param1:String, param2:int, param3:Matrix3D, param4:Boolean = false) : void
    {
        this.context3D.setProgramConstantsFromMatrix(param1,param2,param3,param4);
    }

    public function setProgramConstantsFromVector(param1:String, param2:int, param3:Vector.<Number>, param4:int = -1) : void
    {
        this.context3D.setProgramConstantsFromVector(param1,param2,param3,param4);
    }

    public function createTexture(param1:int, param2:int, param3:String, param4:Boolean) : TextureProxy
    {
        return new TextureProxy(this.context3D.createTexture(param1,param2,param3,param4));
    }

    public function setTextureAt(param1:int, param2:TextureProxy) : void
    {
        this.context3D.setTextureAt(param1,param2.getTexture());
    }

    public function setProgram(param1:Program3DProxy) : void
    {
        this.context3D.setProgram(param1.getProgram3D());
    }

    public function drawTriangles(param1:IndexBuffer3DProxy) : void
    {
        this.context3D.drawTriangles(param1.getIndexBuffer3D());
    }

    public function setBlendFactors(param1:String, param2:String) : void
    {
        this.context3D.setBlendFactors(param1,param2);
    }

    public function setDepthTest(param1:Boolean, param2:String) : void
    {
        this.context3D.setDepthTest(param1,param2);
    }
}
}
